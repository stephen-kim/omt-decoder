use bytes::Bytes;
use libomtnet::{OMTClient, OMTFrame, OMTFrameType};
use tokio::sync::mpsc;

pub struct ReceiverHandle {
    pub audio_rx: mpsc::Receiver<OMTFrame>,
    pub video_rx: mpsc::Receiver<OMTFrame>,
    pub _cancel: tokio::sync::watch::Sender<bool>,
}

/// Start a background task that connects to an OMT source and receives frames.
/// Audio and video are dispatched to separate channels so one never blocks the other.
pub fn start_receiver(address: &str) -> Option<ReceiverHandle> {
    let addr = address
        .strip_prefix("omt://")
        .unwrap_or(address)
        .to_string();

    if addr.is_empty() || addr == "None" {
        return None;
    }

    let (audio_tx, audio_rx) = mpsc::channel::<OMTFrame>(64);
    let (video_tx, video_rx) = mpsc::channel::<OMTFrame>(16);
    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        loop {
            println!("Connecting to {}...", addr);
            let client = OMTClient::connect(&addr).await;
            let mut client = match client {
                Ok(mut c) => {
                    println!("Connected to {}", addr);
                    // Subscribe to video, audio, and metadata
                    if let Err(e) = send_subscribe(&mut c).await {
                        eprintln!("Failed to send subscriptions: {}", e);
                    }
                    c
                }
                Err(e) => {
                    eprintln!("Connection failed: {}", e);
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => continue,
                        _ = cancel_rx.changed() => return,
                    }
                }
            };

            let mut frame_count: u64 = 0;
            let mut audio_count: u64 = 0;
            let mut video_count: u64 = 0;

            loop {
                tokio::select! {
                    frame = client.receive() => {
                        match frame {
                            Some(Ok(f)) => {
                                frame_count += 1;
                                if frame_count <= 5 || frame_count % 500 == 0 {
                                    println!("Frame #{}: type={:?} data_len={}",
                                        frame_count, f.header.frame_type, f.data.len());
                                }

                                let send_result = match f.header.frame_type {
                                    OMTFrameType::Audio => {
                                        audio_count += 1;
                                        audio_tx.try_send(f)
                                    }
                                    OMTFrameType::Video => {
                                        video_count += 1;
                                        if video_count <= 3 {
                                            println!("Video frame #{}: {}x{} codec=0x{:08X}",
                                                video_count,
                                                f.video_header.as_ref().map(|h| h.width).unwrap_or(0),
                                                f.video_header.as_ref().map(|h| h.height).unwrap_or(0),
                                                f.video_header.as_ref().map(|h| h.codec).unwrap_or(0));
                                        }
                                        video_tx.try_send(f)
                                    }
                                    _ => Ok(()),
                                };
                                if let Err(mpsc::error::TrySendError::Closed(_)) = send_result {
                                    return;
                                }
                            }
                            Some(Err(e)) => {
                                eprintln!("Receive error: {}", e);
                                break;
                            }
                            None => {
                                println!("Connection closed (frames: {} audio: {} video: {})",
                                    frame_count, audio_count, video_count);
                                break;
                            }
                        }
                    }
                    _ = cancel_rx.changed() => return,
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {},
                _ = cancel_rx.changed() => return,
            }
        }
    });

    Some(ReceiverHandle {
        audio_rx,
        video_rx,
        _cancel: cancel_tx,
    })
}

/// Build and send an OMT metadata frame with the given XML payload.
fn make_metadata_frame(xml: &str) -> OMTFrame {
    let mut frame = OMTFrame::new(OMTFrameType::Metadata);
    frame.data = Bytes::from(xml.to_string());
    frame.update_data_length();
    frame
}

/// Send subscription messages so the server starts streaming frames.
async fn send_subscribe(client: &mut OMTClient) -> Result<(), std::io::Error> {
    client
        .send(make_metadata_frame(r#"<OMTSubscribe Video="true" />"#))
        .await?;
    client
        .send(make_metadata_frame(r#"<OMTSubscribe Audio="true" />"#))
        .await?;
    client
        .send(make_metadata_frame(r#"<OMTSubscribe Metadata="true" />"#))
        .await?;
    println!("Subscriptions sent");
    Ok(())
}
