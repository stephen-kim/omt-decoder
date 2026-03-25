use bytes::Bytes;
use libomtnet::{OMTClient, OMTFrame, OMTFrameType};
use std::sync::mpsc;

pub struct ReceiverHandle {
    pub audio_rx: mpsc::Receiver<OMTFrame>,
    pub video_rx: mpsc::Receiver<OMTFrame>,
    _thread: std::thread::JoinHandle<()>,
}

/// Start a receiver on a dedicated OS thread with its own tokio runtime.
/// Frames are delivered via std::sync::mpsc — no tokio in the delivery path.
pub fn start_receiver(address: &str) -> Option<ReceiverHandle> {
    let addr = address
        .strip_prefix("omt://")
        .unwrap_or(address)
        .to_string();

    if addr.is_empty() || addr == "None" {
        return None;
    }

    let (audio_tx, audio_rx) = mpsc::sync_channel::<OMTFrame>(64);
    let (video_tx, video_rx) = mpsc::sync_channel::<OMTFrame>(4);

    let thread = std::thread::Builder::new()
        .name("receiver".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create receiver runtime");

            rt.block_on(async move {
                receiver_loop(&addr, audio_tx, video_tx).await;
            });
        })
        .expect("failed to spawn receiver thread");

    Some(ReceiverHandle {
        audio_rx,
        video_rx,
        _thread: thread,
    })
}

async fn receiver_loop(
    addr: &str,
    audio_tx: mpsc::SyncSender<OMTFrame>,
    video_tx: mpsc::SyncSender<OMTFrame>,
) {
    loop {
        println!("Connecting to {}...", addr);
        let client = OMTClient::connect(addr).await;
        let mut client = match client {
            Ok(mut c) => {
                println!("Connected to {}", addr);
                if let Err(e) = send_subscribe(&mut c).await {
                    eprintln!("Failed to send subscriptions: {}", e);
                }
                c
            }
            Err(e) => {
                eprintln!("Connection failed: {}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        loop {
            match client.receive().await {
                Some(Ok(f)) => {
                    let closed = match f.header.frame_type {
                        OMTFrameType::Audio => audio_tx.try_send(f).is_err()
                            && audio_tx.try_send(OMTFrame::new(OMTFrameType::None)).is_err(),
                        OMTFrameType::Video => {
                            // try_send: drop frame if consumer can't keep up
                            let _ = video_tx.try_send(f);
                            false
                        }
                        _ => false,
                    };
                    // If audio channel is disconnected, receiver is being dropped
                    if closed {
                        return;
                    }
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

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

fn make_metadata_frame(xml: &str) -> OMTFrame {
    let mut frame = OMTFrame::new(OMTFrameType::Metadata);
    frame.data = Bytes::from(xml.to_string());
    frame.update_data_length();
    frame
}

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
