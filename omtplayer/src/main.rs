mod discovery;
mod receiver;
mod settings;
mod web_server;

#[cfg(target_os = "linux")]
mod audio;
#[cfg(target_os = "linux")]
mod video;
#[cfg(target_os = "linux")]
mod vmx_decoder;

use anyhow::Result;
use settings::Settings;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    println!("OMT Player (Rust)");

    let config_path = std::path::Path::new(&std::env::current_exe()?)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("config.json");
    let config_path_str = config_path.to_string_lossy().to_string();

    let settings = Settings::load(&config_path_str).unwrap_or_else(|_| {
        println!("Config not found, using defaults.");
        let defaults = Settings::default();
        let _ = defaults.save(&config_path_str);
        defaults
    });

    let shared_settings = Arc::new(RwLock::new(settings.clone()));
    let (settings_tx, mut settings_rx) = watch::channel(settings.clone());

    // Start discovery
    let sources = discovery::start_discovery();

    // Start web server
    let web_state = web_server::WebState {
        settings: shared_settings.clone(),
        settings_tx: settings_tx.clone(),
        config_path: config_path_str.clone(),
        sources: sources.clone(),
    };
    let web_port = settings.web_port;
    tokio::spawn(async move {
        if let Err(e) = web_server::start_web_server(web_port, web_state).await {
            eprintln!("Web server error: {}", e);
        }
    });

    println!("Web server on port {}", web_port);

    // Main receive loop
    let mut current_source = settings.source.clone();
    let mut recv_handle: Option<receiver::ReceiverHandle> = None;

    if current_source != "None" && !current_source.is_empty() {
        recv_handle = receiver::start_receiver(&current_source);
    }

    #[cfg(target_os = "linux")]
    let mut audio_player = audio::AudioPlayer::new(&settings.audio_devices);

    #[cfg(target_os = "linux")]
    let mut video_output: Option<video::VideoOutput> = None;
    #[cfg(target_os = "linux")]
    let mut vmx_dec: Option<vmx_decoder::VmxDecoder> = None;
    #[cfg(target_os = "linux")]
    let mut current_width: u32 = 0;
    #[cfg(target_os = "linux")]
    let mut current_height: u32 = 0;

    loop {
        tokio::select! {
            Ok(()) = settings_rx.changed() => {
                let new_settings = settings_rx.borrow_and_update().clone();

                // Source changed?
                if new_settings.source != current_source {
                    println!("Source changed: {}", new_settings.source);
                    current_source = new_settings.source.clone();
                    recv_handle = None;
                    if current_source != "None" && !current_source.is_empty() {
                        recv_handle = receiver::start_receiver(&current_source);
                    }
                }

                // Audio devices changed?
                #[cfg(target_os = "linux")]
                {
                    audio_player.set_devices(&new_settings.audio_devices);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Shutting down...");
                break;
            }
            frame = async {
                match &mut recv_handle {
                    Some(h) => h.recv().await,
                    None => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        None
                    }
                }
            } => {
                if let Some(frame) = frame {
                    match frame.header.frame_type {
                        #[cfg(target_os = "linux")]
                        libomtnet::OMTFrameType::Audio => {
                            if let Some(ref audio_header) = frame.audio_header {
                                audio_player.enqueue(
                                    &frame.data,
                                    audio_header.channels as u32,
                                    audio_header.samples_per_channel as u32,
                                    audio_header.sample_rate as u32,
                                );
                            }
                        }
                        #[cfg(target_os = "linux")]
                        libomtnet::OMTFrameType::Video => {
                            if let Some(ref video_header) = frame.video_header {
                                let w = video_header.width as u32;
                                let h = video_header.height as u32;

                                // Resolution changed? Recreate decoder + presenter
                                if w != current_width || h != current_height {
                                    current_width = w;
                                    current_height = h;
                                    let frame_rate = if video_header.frame_rate_d > 0 {
                                        video_header.frame_rate_n as f32 / video_header.frame_rate_d as f32
                                    } else {
                                        60.0
                                    };
                                    println!("New format: {}x{} @ {:.2}fps", w, h, frame_rate);
                                    vmx_dec = vmx_decoder::VmxDecoder::new(w, h);
                                    video_output = video::VideoOutput::new(w, h, frame_rate);
                                }

                                // Decode VMX1 → BGRA
                                if let Some(ref mut dec) = vmx_dec {
                                    if let Some(bgra_data) = dec.decode(&frame.data) {
                                        if let Some(ref mut vo) = video_output {
                                            vo.present(&bgra_data, w * 4);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Save settings on exit
    let final_settings = shared_settings.read().await;
    if let Err(e) = final_settings.save(&config_path_str) {
        eprintln!("Failed to save settings: {}", e);
    }

    Ok(())
}
