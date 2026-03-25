use bytes::{Buf, Bytes, BytesMut};
use libomtnet::{OMTAudioHeader, OMTFrame, OMTFrameHeader, OMTFrameType, OMTVideoHeader};
use std::io::{self, Read, Write};
use std::net::TcpStream;

/// Blocking OMT frame reader — no channels, no async.
/// Call next_frame() in a loop, just like C# OMTReceive.Receive().
pub struct OMTConnection {
    stream: TcpStream,
    buf: BytesMut,
    read_buf: Vec<u8>,
}

impl OMTConnection {
    pub fn connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        let _ = set_socket_buffers(&stream);
        send_subscribe(&stream)?;
        Ok(OMTConnection {
            stream,
            buf: BytesMut::with_capacity(2 * 1024 * 1024),
            read_buf: vec![0u8; 512 * 1024],
        })
    }

    /// Read the next complete OMT frame. Blocks until a frame is available.
    pub fn next_frame(&mut self) -> io::Result<OMTFrame> {
        loop {
            // Try to parse a frame from buffered data
            if let Some(frame) = self.try_parse_frame()? {
                return Ok(frame);
            }
            // Need more data
            let n = self.stream.read(&mut self.read_buf)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
            }
            self.buf.extend_from_slice(&self.read_buf[..n]);
        }
    }

    fn try_parse_frame(&mut self) -> io::Result<Option<OMTFrame>> {
        if self.buf.len() < OMTFrameHeader::SIZE {
            return Ok(None);
        }

        let header = {
            let mut cursor = io::Cursor::new(&self.buf[..]);
            OMTFrameHeader::read(&mut cursor)?
        };

        let total = OMTFrameHeader::SIZE + header.data_length as usize;
        if self.buf.len() < total {
            return Ok(None);
        }

        self.buf.advance(OMTFrameHeader::SIZE);

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
                frame.video_header = Some(OMTVideoHeader::read(&mut self.buf)?);
                OMTVideoHeader::SIZE
            }
            OMTFrameType::Audio => {
                frame.audio_header = Some(OMTAudioHeader::read(&mut self.buf)?);
                OMTAudioHeader::SIZE
            }
            _ => 0,
        };

        let payload_len = header.data_length as usize - ext_size;
        let meta_len = (header.metadata_length as usize).min(payload_len);
        let data_len = payload_len - meta_len;

        if data_len > 0 {
            frame.data = self.buf.split_to(data_len).freeze();
        }
        if meta_len > 0 {
            frame.metadata = self.buf.split_to(meta_len).freeze();
        }

        Ok(Some(frame))
    }

    /// Set a read timeout on the underlying socket.
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }
}

#[cfg(target_os = "linux")]
fn set_socket_buffers(stream: &TcpStream) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    let recv_buf: libc::c_int = 8 * 1024 * 1024;
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
    let mut header = [0u8; 16];
    header[0] = 1;
    header[1] = OMTFrameType::Metadata as u8;
    header[12..16].copy_from_slice(&data_len.to_le_bytes());
    stream.write_all(&header)?;
    stream.write_all(data)?;
    stream.flush()?;
    Ok(())
}
