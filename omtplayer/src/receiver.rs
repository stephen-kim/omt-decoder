use libomtnet::{OMTClient, OMTFrame};
use tokio::sync::mpsc;

pub struct ReceiverHandle {
    rx: mpsc::Receiver<OMTFrame>,
    _cancel: tokio::sync::watch::Sender<bool>,
}

impl ReceiverHandle {
    pub async fn recv(&mut self) -> Option<OMTFrame> {
        self.rx.recv().await
    }
}

/// Start a background task that connects to an OMT source and receives frames.
/// Returns a handle to receive frames, or None if the address is invalid.
pub fn start_receiver(address: &str) -> Option<ReceiverHandle> {
    // Parse omt://host:port → host:port
    let addr = address
        .strip_prefix("omt://")
        .unwrap_or(address)
        .to_string();

    if addr.is_empty() || addr == "None" {
        return None;
    }

    let (tx, rx) = mpsc::channel::<OMTFrame>(64);
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
                                if tx.send(f).await.is_err() {
                                    return; // receiver dropped
                                }
                            }
                            Some(Err(e)) => {
                                eprintln!("Receive error: {}", e);
                                break; // reconnect
                            }
                            None => {
                                println!("Connection closed");
                                break; // reconnect
                            }
                        }
                    }
                    _ = cancel_rx.changed() => return,
                }
            }

            // Brief delay before reconnect
            tokio::select! {
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {},
                _ = cancel_rx.changed() => return,
            }
        }
    });

    Some(ReceiverHandle {
        rx,
        _cancel: cancel_tx,
    })
}
