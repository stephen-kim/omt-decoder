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
    let (video_tx, video_rx) = mpsc::channel::<OMTFrame>(8);
    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        loop {
            println!("Connecting to {}...", addr);
            let client = OMTClient::connect(&addr).await;
            let mut client = match client {
                Ok(c) => {
                    println!("Connected to {}", addr);
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

            loop {
                tokio::select! {
                    frame = client.receive() => {
                        match frame {
                            Some(Ok(f)) => {
                                let send_result = match f.header.frame_type {
                                    OMTFrameType::Audio => audio_tx.try_send(f),
                                    OMTFrameType::Video => video_tx.try_send(f),
                                    _ => Ok(()),
                                };
                                if let Err(mpsc::error::TrySendError::Closed(_)) = send_result {
                                    return;
                                }
                                // TrySendError::Full is fine — drop the frame rather than block
                            }
                            Some(Err(e)) => {
                                eprintln!("Receive error: {}", e);
                                break;
                            }
                            None => {
                                println!("Connection closed");
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
