#![allow(unused_variables)]

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
    let (settings_tx, settings_rx) = watch::channel(settings.clone());

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

    // Spawn the player loop on a dedicated OS thread (matches C# synchronous main loop)
    let player_settings = settings.clone();
    let player_settings_rx = settings_rx.clone();
    let player_handle = std::thread::Builder::new()
        .name("player".into())
        .spawn(move || {
            player_loop(player_settings, player_settings_rx);
        })
        .expect("failed to spawn player thread");

    // Main tokio thread just waits for shutdown
    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");

    // Save settings on exit
    let final_settings = shared_settings.read().await;
    if let Err(e) = final_settings.save(&config_path_str) {
        eprintln!("Failed to save settings: {}", e);
    }

    Ok(())
}

/// Synchronous player loop — mirrors the C# Program.Main() while loop.
/// Runs on its own OS thread, no tokio involved.
fn player_loop(initial_settings: Settings, mut settings_rx: watch::Receiver<Settings>) {
    let mut current_source = initial_settings.source.clone();
    let mut recv: Option<receiver::ReceiverHandle> = None;

    #[cfg(target_os = "linux")]
    let mut audio_player = audio::AudioPlayer::new(&initial_settings.audio_devices);

    #[cfg(target_os = "linux")]
    let mut video_output: Option<video::VideoOutput> = None;
    #[cfg(target_os = "linux")]
    let mut vmx_dec: Option<vmx_decoder::VmxDecoder> = None;
    #[cfg(target_os = "linux")]
    let mut current_width: u32 = 0;
    #[cfg(target_os = "linux")]
    let mut current_height: u32 = 0;

    if current_source != "None" && !current_source.is_empty() {
        recv = receiver::start_receiver(&current_source);
    }

    let mut frame_count: u64 = 0;
    let mut fps_timer = std::time::Instant::now();

    loop {
        // Check for settings changes (non-blocking)
        if settings_rx.has_changed().unwrap_or(false) {
            let new_settings = settings_rx.borrow_and_update().clone();

            if new_settings.source != current_source {
                println!("Source changed: {}", new_settings.source);
                current_source = new_settings.source.clone();
                recv = None; // drop old receiver
                #[cfg(target_os = "linux")]
                {
                    video_output = None;
                    vmx_dec = None;
                    current_width = 0;
                    current_height = 0;
                }
                if current_source != "None" && !current_source.is_empty() {
                    recv = receiver::start_receiver(&current_source);
                }
            }

            #[cfg(target_os = "linux")]
            {
                audio_player.set_devices(&new_settings.audio_devices);
            }
        }

        let Some(ref r) = recv else {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        };

        // Audio: drain all available frames (non-blocking)
        #[cfg(target_os = "linux")]
        while let Ok(frame) = r.audio_rx.try_recv() {
            if let Some(ref ah) = frame.audio_header {
                audio_player.enqueue(
                    &frame.data,
                    ah.channels as u32,
                    ah.samples_per_channel as u32,
                    ah.sample_rate as u32,
                );
            }
        }

        // Video: block until next frame (matching C# Receive with timeout)
        let video_frame = r.video_rx.recv().ok();

        #[cfg(target_os = "linux")]
        if let Some(frame) = video_frame {
            if let Some(ref vh) = frame.video_header {
                let w = vh.width as u32;
                let h = vh.height as u32;

                if w != current_width || h != current_height {
                    current_width = w;
                    current_height = h;
                    let frame_rate = if vh.frame_rate_d > 0 {
                        vh.frame_rate_n as f32 / vh.frame_rate_d as f32
                    } else {
                        60.0
                    };
                    println!(
                        "Video: {}x{} @ {:.2}fps codec=0x{:08X}",
                        w, h, frame_rate, vh.codec
                    );
                    vmx_dec = vmx_decoder::VmxDecoder::new(w, h);
                    if vmx_dec.is_none() {
                        eprintln!("Video: VMX decoder creation failed");
                    }
                    video_output = video::VideoOutput::new(w, h, frame_rate);
                    if video_output.is_none() {
                        eprintln!("Video: display output creation failed");
                    }
                }

                if let Some(ref mut dec) = vmx_dec {
                    let t0 = std::time::Instant::now();
                    if let Some(bgra_data) = dec.decode(&frame.data) {
                        let decode_ms = t0.elapsed().as_millis();
                        let t1 = std::time::Instant::now();
                        if let Some(ref mut vo) = video_output {
                            vo.present(bgra_data, w * 4);
                        }
                        let present_ms = t1.elapsed().as_millis();
                        frame_count += 1;
                        if frame_count % 300 == 0 {
                            let elapsed = fps_timer.elapsed().as_secs_f64();
                            let fps = 300.0 / elapsed;
                            println!(
                                "Frame {}: decode={}ms present={}ms fps={:.1}",
                                frame_count, decode_ms, present_ms, fps
                            );
                            fps_timer = std::time::Instant::now();
                        }
                    }
                }
            }
        }
    }
}
