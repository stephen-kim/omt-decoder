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
#[cfg(target_os = "linux")]
mod hw_decoder;

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
    let (settings_tx, settings_rx) = watch::channel(settings.clone());

    let sources = discovery::start_discovery();

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

    let player_settings = settings.clone();
    let player_settings_rx = settings_rx.clone();
    std::thread::Builder::new()
        .name("player".into())
        .spawn(move || {
            player_loop(player_settings, player_settings_rx);
        })
        .expect("failed to spawn player thread");

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");

    let final_settings = shared_settings.read().await;
    if let Err(e) = final_settings.save(&config_path_str) {
        eprintln!("Failed to save settings: {}", e);
    }

    Ok(())
}

/// Player thread: TCP read → audio enqueue inline → video to decode thread.
/// Minimal latency: audio written to ALSA as soon as parsed from TCP,
/// video sent to decode thread with 2-frame buffer.
fn player_loop(initial_settings: Settings, mut settings_rx: watch::Receiver<Settings>) {
    #[cfg(target_os = "linux")]
    let mut audio_player =
        audio::AudioPlayer::new(&initial_settings.audio_devices, initial_settings.volume);

    let mut current_source = initial_settings.source.clone();
    let mut current_quality = initial_settings.quality.clone();
    let mut current_codec = initial_settings.codec.clone();
    let mut conn: Option<receiver::OMTConnection> = None;

    let (video_tx, video_rx) = std::sync::mpsc::sync_channel::<libomtnet::OMTFrame>(2);
    std::thread::Builder::new()
        .name("video".into())
        .spawn(move || video_thread(video_rx))
        .expect("failed to spawn video thread");

    if current_source != "None" && !current_source.is_empty() {
        conn = try_connect(&current_source, &current_quality, &current_codec);
    }

    loop {
        if settings_rx.has_changed().unwrap_or(false) {
            let new_settings = settings_rx.borrow_and_update().clone();

            if new_settings.source != current_source {
                println!("Source changed: {}", new_settings.source);
                current_source = new_settings.source.clone();
                current_quality = new_settings.quality.clone();
                current_codec = new_settings.codec.clone();
                conn = None;
                if current_source != "None" && !current_source.is_empty() {
                    conn = try_connect(&current_source, &current_quality, &current_codec);
                }
            } else if new_settings.quality != current_quality || new_settings.codec != current_codec {
                current_quality = new_settings.quality.clone();
                current_codec = new_settings.codec.clone();
                println!("Settings changed: quality={} codec={}", current_quality, current_codec);
                if let Some(ref c) = conn {
                    let _ = c.send_settings(&current_quality, &current_codec);
                }
            }

            #[cfg(target_os = "linux")]
            {
                audio_player.set_devices(&new_settings.audio_devices);
                audio_player.set_volume(new_settings.volume);
            }
        }

        let Some(ref mut c) = conn else {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        };

        // Audio processed inline during TCP read; only video frames returned
        #[cfg(target_os = "linux")]
        let frame = {
            let ap = &mut audio_player;
            c.next_video_frame(|f| {
                if let Some(ref ah) = f.audio_header {
                    ap.enqueue(&f.data, ah.channels as u32, ah.samples_per_channel as u32, ah.sample_rate as u32);
                }
            })
        };
        #[cfg(not(target_os = "linux"))]
        let frame = c.next_frame();

        match frame {
            Ok(f) => { let _ = video_tx.try_send(f); }
            Err(e) => {
                eprintln!("Connection lost: {}", e);
                conn = None;
                std::thread::sleep(std::time::Duration::from_secs(1));
                if current_source != "None" && !current_source.is_empty() {
                    conn = try_connect(&current_source, &current_quality, &current_codec);
                }
            }
        }
    }
}

/// Video decode + DRM present on dedicated thread.
/// Routes to VMX1 software decoder or H.264/H.265 hardware decoder based on codec field.
#[cfg(target_os = "linux")]
fn video_thread(rx: std::sync::mpsc::Receiver<libomtnet::OMTFrame>) {
    use libomtnet::OMTCodec;

    let mut video_output: Option<video::VideoOutput> = None;
    let mut vmx_dec: Option<vmx_decoder::VmxDecoder> = None;
    let mut hw_dec: Option<hw_decoder::HwDecoder> = None;
    let mut current_width: u32 = 0;
    let mut current_height: u32 = 0;
    let mut current_codec: i32 = 0;

    while let Ok(frame) = rx.recv() {
        if let Some(ref vh) = frame.video_header {
            let w = vh.width as u32;
            let h = vh.height as u32;

            if w != current_width || h != current_height || vh.codec != current_codec {
                current_width = w;
                current_height = h;
                current_codec = vh.codec;
                let frame_rate = if vh.frame_rate_d > 0 {
                    vh.frame_rate_n as f32 / vh.frame_rate_d as f32
                } else {
                    60.0
                };

                let codec_name = match vh.codec {
                    c if c == OMTCodec::VMX1 as i32 => "VMX1",
                    c if c == OMTCodec::H264 as i32 => "H.264",
                    c if c == OMTCodec::H265 as i32 => "H.265",
                    c if c == OMTCodec::BGRA as i32 => "BGRA",
                    _ => "unknown",
                };
                println!("Video: {}x{} @ {:.2}fps codec={}", w, h, frame_rate, codec_name);

                // Reset decoders
                vmx_dec = None;
                hw_dec = None;
                match vh.codec {
                    c if c == OMTCodec::VMX1 as i32 => {
                        vmx_dec = vmx_decoder::VmxDecoder::new(w, h);
                    }
                    c if c == OMTCodec::H264 as i32 => {
                        hw_dec = hw_decoder::HwDecoder::new(w, h, "H264");
                        if hw_dec.is_none() {
                            eprintln!("Video: H.264 hardware decoder not available");
                        }
                    }
                    c if c == OMTCodec::H265 as i32 => {
                        hw_dec = hw_decoder::HwDecoder::new(w, h, "H265");
                        if hw_dec.is_none() {
                            eprintln!("Video: H.265 hardware decoder not available");
                        }
                    }
                    _ => {}
                }
                video_output = video::VideoOutput::new(w, h, frame_rate);
            }

            // Decode and present
            let bgra: Option<&[u8]> = match vh.codec {
                c if c == OMTCodec::VMX1 as i32 => {
                    vmx_dec.as_mut().and_then(|d| d.decode(&frame.data))
                }
                c if c == OMTCodec::H264 as i32 || c == OMTCodec::H265 as i32 => {
                    hw_dec.as_mut().and_then(|d| d.decode(&frame.data))
                }
                c if c == OMTCodec::BGRA as i32 => {
                    Some(&frame.data)
                }
                _ => None,
            };

            if let Some(pixels) = bgra {
                if let Some(ref mut vo) = video_output {
                    vo.present(pixels, w * 4);
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn video_thread(rx: std::sync::mpsc::Receiver<libomtnet::OMTFrame>) {
    while let Ok(_) = rx.recv() {}
}

fn try_connect(source: &str, quality: &str, codec: &str) -> Option<receiver::OMTConnection> {
    let addr = source.strip_prefix("omt://").unwrap_or(source);
    println!("Connecting to {}...", addr);
    match receiver::OMTConnection::connect(addr, quality, codec) {
        Ok(c) => {
            println!("Connected, subscriptions sent");
            Some(c)
        }
        Err(e) => {
            eprintln!("Connection failed: {}", e);
            None
        }
    }
}
