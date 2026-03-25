use bytes::{Buf, Bytes, BytesMut};
use libomtnet::{
    OMTAudioHeader, OMTFrame, OMTFrameHeader, OMTFrameType, OMTVideoHeader,
};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;

pub struct ReceiverHandle {
    pub audio_rx: mpsc::Receiver<OMTFrame>,
    pub video_rx: mpsc::Receiver<OMTFrame>,
    _thread: std::thread::JoinHandle<()>,
}

/// Start a receiver on a dedicated OS thread using blocking TCP.
/// No tokio involved — pure std::net for minimal overhead.
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
            receiver_loop(&addr, audio_tx, video_tx);
        })
        .expect("failed to spawn receiver thread");

    Some(ReceiverHandle {
        audio_rx,
        video_rx,
        _thread: thread,
    })
}

fn receiver_loop(
    addr: &str,
    audio_tx: mpsc::SyncSender<OMTFrame>,
    video_tx: mpsc::SyncSender<OMTFrame>,
) {
    loop {
        println!("Connecting to {}...", addr);
        let stream = match TcpStream::connect(addr) {
            Ok(s) => {
                println!("Connected to {}", addr);
                // Set socket buffer sizes
                let _ = set_socket_buffers(&s);
                s
            }
            Err(e) => {
                eprintln!("Connection failed: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        };

        // Send subscriptions
        if let Err(e) = send_subscribe(&stream) {
            eprintln!("Failed to send subscriptions: {}", e);
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        println!("Subscriptions sent");

        // Read frames
        if let Err(e) = read_frames(stream, &audio_tx, &video_tx) {
            eprintln!("Connection error: {}", e);
        }

        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn read_frames(
    mut stream: TcpStream,
    audio_tx: &mpsc::SyncSender<OMTFrame>,
    video_tx: &mpsc::SyncSender<OMTFrame>,
) -> io::Result<()> {
    let mut buf = BytesMut::with_capacity(1024 * 1024);
    let mut read_buf = vec![0u8; 256 * 1024];

    loop {
        // Read more data from TCP
        let n = stream.read(&mut read_buf)?;
        if n == 0 {
            println!("Connection closed");
            return Ok(());
        }
        buf.extend_from_slice(&read_buf[..n]);

        // Parse all complete frames in buffer
        while buf.len() >= OMTFrameHeader::SIZE {
            // Peek header
            let header = {
                let mut cursor = io::Cursor::new(&buf[..]);
                OMTFrameHeader::read(&mut cursor)?
            };

            let total = OMTFrameHeader::SIZE + header.data_length as usize;
            if buf.len() < total {
                break; // need more data
            }

            // Consume header
            buf.advance(OMTFrameHeader::SIZE);

            // Parse frame
            let mut frame = OMTFrame {
                header: header.clone(),
                video_header: None,
                audio_header: None,
                metadata: Bytes::new(),
                data: Bytes::new(),
                preview_mode: false,
                preview_data_length: None,
            };

            let ext_size = match header.frame_type {
                OMTFrameType::Video => {
                    frame.video_header = Some(OMTVideoHeader::read(&mut buf)?);
                    OMTVideoHeader::SIZE
                }
                OMTFrameType::Audio => {
                    frame.audio_header = Some(OMTAudioHeader::read(&mut buf)?);
                    OMTAudioHeader::SIZE
                }
                _ => 0,
            };

            let payload_len = header.data_length as usize - ext_size;
            let meta_len = (header.metadata_length as usize).min(payload_len);
            let data_len = payload_len - meta_len;

            if data_len > 0 {
                frame.data = buf.split_to(data_len).freeze();
            }
            if meta_len > 0 {
                frame.metadata = buf.split_to(meta_len).freeze();
            }

            // Dispatch
            match frame.header.frame_type {
                OMTFrameType::Audio => {
                    let _ = audio_tx.try_send(frame);
                }
                OMTFrameType::Video => {
                    let _ = video_tx.try_send(frame);
                }
                _ => {}
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn set_socket_buffers(stream: &TcpStream) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    let recv_buf: libc::c_int = 8 * 1024 * 1024; // 8MB
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &recv_buf as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn set_socket_buffers(_stream: &TcpStream) -> io::Result<()> {
    Ok(())
}

fn send_subscribe(stream: &TcpStream) -> io::Result<()> {
    let mut stream = stream;
    send_metadata_frame(&mut stream, r#"<OMTSubscribe Video="true" />"#)?;
    send_metadata_frame(&mut stream, r#"<OMTSubscribe Audio="true" />"#)?;
    send_metadata_frame(&mut stream, r#"<OMTSubscribe Metadata="true" />"#)?;
    Ok(())
}

fn send_metadata_frame(stream: &mut &TcpStream, xml: &str) -> io::Result<()> {
    let data = xml.as_bytes();
    let data_len = data.len() as i32;

    // Write OMT header: version(1) + type(1) + timestamp(8) + metadata_len(2) + data_len(4) = 16 bytes
    let mut header = [0u8; 16];
    header[0] = 1; // version
    header[1] = OMTFrameType::Metadata as u8;
    // timestamp = 0 (bytes 2-9)
    // metadata_length = 0 (bytes 10-11)
    header[12..16].copy_from_slice(&data_len.to_le_bytes());

    stream.write_all(&header)?;
    stream.write_all(data)?;
    stream.flush()?;
    Ok(())
}
