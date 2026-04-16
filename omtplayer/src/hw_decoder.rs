//! V4L2 M2M hardware decoder for H.264/H.265 All-Intra streams.
//! Outputs BGRA for DRM display.

use std::collections::VecDeque;
use std::fs;
use std::os::fd::RawFd;

// ── V4L2 constants ────────────────────────────────────────────────────────

const V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE: u32 = 8;
const V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE: u32 = 9;
const V4L2_MEMORY_MMAP: u32 = 1;
const V4L2_FIELD_NONE: u32 = 1;

// Pixel formats (fourcc)
const V4L2_PIX_FMT_H264: u32 = fourcc(b"H264");
const V4L2_PIX_FMT_HEVC: u32 = fourcc(b"HEVC");
const V4L2_PIX_FMT_NV12: u32 = fourcc(b"NV12");
const V4L2_PIX_FMT_YUV420: u32 = fourcc(b"YU12");

const VIDIOC_QUERYCAP: libc::c_ulong = 0x80685600;
const VIDIOC_ENUM_FMT: libc::c_ulong = 0xC0405602;
const VIDIOC_S_FMT: libc::c_ulong = 0xC0D05605;
const VIDIOC_REQBUFS: libc::c_ulong = 0xC0145608;
const VIDIOC_QUERYBUF: libc::c_ulong = 0xC0445609;
const VIDIOC_QBUF: libc::c_ulong = 0xC044560F;
const VIDIOC_DQBUF: libc::c_ulong = 0xC0445611;
const VIDIOC_STREAMON: libc::c_ulong = 0x40045612;
const VIDIOC_STREAMOFF: libc::c_ulong = 0x40045613;

const V4L2_CAP_VIDEO_M2M_MPLANE: u32 = 0x00004000;
const V4L2_CAP_STREAMING: u32 = 0x04000000;

const NUM_OUTPUT_BUFS: u32 = 4;
const NUM_CAPTURE_BUFS: u32 = 4;
const COMPRESSED_BUF_SIZE: u32 = 2 * 1024 * 1024; // 2MB per compressed frame

const fn fourcc(b: &[u8; 4]) -> u32 {
    (b[0] as u32) | ((b[1] as u32) << 8) | ((b[2] as u32) << 16) | ((b[3] as u32) << 24)
}

// ── V4L2 structs ──────────────────────────────────────────────────────────

#[repr(C)]
struct v4l2_capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Default)]
struct v4l2_fmtdesc {
    index: u32,
    buf_type: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct v4l2_pix_format_mplane {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    colorspace: u32,
    plane_fmt: [v4l2_plane_pix_format; 8],
    num_planes: u8,
    flags: u8,
    _ycbcr_enc_or_hsv_enc: u8,
    quantization: u8,
    xfer_func: u8,
    reserved: [u8; 7],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct v4l2_plane_pix_format {
    sizeimage: u32,
    bytesperline: u32,
    reserved: [u16; 6],
}

// v4l2_format with pix_mp variant
#[repr(C)]
struct v4l2_format {
    buf_type: u32,
    pix_mp: v4l2_pix_format_mplane,
}

#[repr(C)]
struct v4l2_requestbuffers {
    count: u32,
    buf_type: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
    reserved2: [u32; 3], // padding to match kernel struct
}

#[repr(C)]
#[derive(Clone, Copy)]
struct v4l2_plane {
    bytesused: u32,
    length: u32,
    m_offset: u32, // union: mem_offset for MMAP
    _padding: u32,
    data_offset: u32,
    reserved: [u32; 11],
}

#[repr(C)]
struct v4l2_buffer {
    index: u32,
    buf_type: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp_sec: i64,
    timestamp_usec: i64,
    timecode: [u32; 4], // struct v4l2_timecode
    sequence: u32,
    memory: u32,
    m_offset: u32, // union
    length: u32,
    reserved2: u32,
    _union2: u32,
    m_planes: *mut v4l2_plane, // for mplane
}

// ── Buffer mapping ────────────────────────────────────────────────────────

struct MmapBuffer {
    ptr: *mut u8,
    length: usize,
}

impl Drop for MmapBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.length); }
        }
    }
}

// ── Hardware Decoder ──────────────────────────────────────────────────────

pub struct HwDecoder {
    fd: RawFd,
    width: u32,
    height: u32,
    output_bufs: Vec<MmapBuffer>,
    capture_bufs: Vec<MmapBuffer>,
    output_free: VecDeque<u32>,
    bgra_buf: Vec<u8>,
    capture_bytesperline: u32,
    capture_height: u32,
}

impl HwDecoder {
    /// Try to open a V4L2 M2M decoder for the given codec.
    /// codec: "H264" or "H265"
    pub fn new(width: u32, height: u32, codec: &str) -> Option<Self> {
        let pix_fmt = match codec {
            "H264" => V4L2_PIX_FMT_H264,
            "H265" => V4L2_PIX_FMT_HEVC,
            _ => return None,
        };

        // Find a V4L2 M2M device that supports the codec
        let fd = find_m2m_device(pix_fmt)?;
        println!("HwDecoder: opened V4L2 M2M device for {}", codec);

        // Set OUTPUT format (compressed)
        if !set_output_format(fd, pix_fmt, width, height) {
            eprintln!("HwDecoder: failed to set output format");
            unsafe { libc::close(fd); }
            return None;
        }

        // Set CAPTURE format (NV12)
        let (cap_bpl, cap_h) = set_capture_format(fd, width, height)?;

        // Allocate OUTPUT buffers
        let output_bufs = alloc_buffers(fd, V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, NUM_OUTPUT_BUFS, COMPRESSED_BUF_SIZE)?;

        // Allocate CAPTURE buffers
        let capture_size = cap_bpl * cap_h * 3 / 2; // NV12: Y + UV
        let capture_bufs = alloc_buffers(fd, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, NUM_CAPTURE_BUFS, capture_size)?;

        // Queue all capture buffers
        for i in 0..capture_bufs.len() as u32 {
            queue_capture_buf(fd, i, capture_bufs[i as usize].length);
        }

        // Stream on
        let mut out_type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        let mut cap_type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        unsafe {
            libc::ioctl(fd, VIDIOC_STREAMON, &mut out_type as *mut u32);
            libc::ioctl(fd, VIDIOC_STREAMON, &mut cap_type as *mut u32);
        }

        let mut output_free = VecDeque::new();
        for i in 0..output_bufs.len() as u32 {
            output_free.push_back(i);
        }

        let aligned_height = (height + 15) & !15;
        let bgra_size = (width * aligned_height * 4) as usize;

        Some(HwDecoder {
            fd,
            width,
            height,
            output_bufs,
            capture_bufs,
            output_free,
            bgra_buf: vec![0u8; bgra_size],
            capture_bytesperline: cap_bpl,
            capture_height: cap_h,
        })
    }

    /// Decode a compressed frame. Returns BGRA slice.
    pub fn decode(&mut self, compressed: &[u8]) -> Option<&[u8]> {
        if compressed.is_empty() {
            return None;
        }

        // Get a free output buffer
        if self.output_free.is_empty() {
            // Try to dequeue a used output buffer
            if let Some(idx) = dequeue_output_buf(self.fd) {
                self.output_free.push_back(idx);
            } else {
                return None; // no buffers available
            }
        }

        let out_idx = self.output_free.pop_front()? as usize;
        let out_buf = &self.output_bufs[out_idx];

        // Copy compressed data to output buffer
        let copy_len = compressed.len().min(out_buf.length);
        unsafe {
            std::ptr::copy_nonoverlapping(compressed.as_ptr(), out_buf.ptr, copy_len);
        }

        // Queue output buffer with data
        queue_output_buf(self.fd, out_idx as u32, copy_len as u32);

        // Dequeue capture buffer (decoded frame)
        let cap_idx = dequeue_capture_buf(self.fd)?;
        let cap_buf = &self.capture_bufs[cap_idx as usize];

        // Convert NV12 → BGRA
        nv12_to_bgra(
            cap_buf.ptr,
            self.capture_bytesperline,
            self.width,
            self.height,
            &mut self.bgra_buf,
        );

        // Re-queue capture buffer
        queue_capture_buf(self.fd, cap_idx, self.capture_bufs[cap_idx as usize].length);

        // Dequeue the output buffer we just used
        if let Some(idx) = dequeue_output_buf(self.fd) {
            self.output_free.push_back(idx);
        }

        Some(&self.bgra_buf)
    }
}

impl Drop for HwDecoder {
    fn drop(&mut self) {
        let mut out_type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        let mut cap_type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        unsafe {
            libc::ioctl(self.fd, VIDIOC_STREAMOFF, &mut out_type as *mut u32);
            libc::ioctl(self.fd, VIDIOC_STREAMOFF, &mut cap_type as *mut u32);
            libc::close(self.fd);
        }
    }
}

// ── V4L2 helpers ──────────────────────────────────────────────────────────

fn find_m2m_device(pix_fmt: u32) -> Option<RawFd> {
    for i in 0..20 {
        let path = format!("/dev/video{}\0", i);
        let fd = unsafe { libc::open(path.as_ptr() as *const i8, libc::O_RDWR | libc::O_NONBLOCK) };
        if fd < 0 { continue; }

        // Check capabilities
        let mut cap: v4l2_capability = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) } != 0 {
            unsafe { libc::close(fd); }
            continue;
        }

        let caps = if cap.device_caps != 0 { cap.device_caps } else { cap.capabilities };
        if caps & V4L2_CAP_VIDEO_M2M_MPLANE == 0 || caps & V4L2_CAP_STREAMING == 0 {
            unsafe { libc::close(fd); }
            continue;
        }

        // Check if OUTPUT supports the codec
        let mut found = false;
        for idx in 0..32 {
            let mut desc = v4l2_fmtdesc {
                index: idx,
                buf_type: V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE,
                ..Default::default()
            };
            if unsafe { libc::ioctl(fd, VIDIOC_ENUM_FMT, &mut desc) } != 0 {
                break;
            }
            if desc.pixelformat == pix_fmt {
                found = true;
                break;
            }
        }

        if found {
            // Switch to blocking mode for decode
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
            return Some(fd);
        }
        unsafe { libc::close(fd); }
    }
    None
}

fn set_output_format(fd: RawFd, pix_fmt: u32, width: u32, height: u32) -> bool {
    let mut fmt: v4l2_format = unsafe { std::mem::zeroed() };
    fmt.buf_type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    fmt.pix_mp.width = width;
    fmt.pix_mp.height = height;
    fmt.pix_mp.pixelformat = pix_fmt;
    fmt.pix_mp.field = V4L2_FIELD_NONE;
    fmt.pix_mp.num_planes = 1;
    fmt.pix_mp.plane_fmt[0].sizeimage = COMPRESSED_BUF_SIZE;

    unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &mut fmt) == 0 }
}

fn set_capture_format(fd: RawFd, width: u32, height: u32) -> Option<(u32, u32)> {
    let mut fmt: v4l2_format = unsafe { std::mem::zeroed() };
    fmt.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    fmt.pix_mp.width = width;
    fmt.pix_mp.height = height;
    fmt.pix_mp.pixelformat = V4L2_PIX_FMT_NV12;
    fmt.pix_mp.field = V4L2_FIELD_NONE;
    fmt.pix_mp.num_planes = 1;

    if unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &mut fmt) } != 0 {
        // Try YUV420 fallback
        fmt.pix_mp.pixelformat = V4L2_PIX_FMT_YUV420;
        if unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &mut fmt) } != 0 {
            eprintln!("HwDecoder: failed to set capture format");
            return None;
        }
    }

    let bpl = if fmt.pix_mp.plane_fmt[0].bytesperline > 0 {
        fmt.pix_mp.plane_fmt[0].bytesperline
    } else {
        width
    };
    let h = fmt.pix_mp.height;
    println!("HwDecoder: capture format {}x{} bpl={}", fmt.pix_mp.width, h, bpl);
    Some((bpl, h))
}

fn alloc_buffers(fd: RawFd, buf_type: u32, count: u32, min_size: u32) -> Option<Vec<MmapBuffer>> {
    let mut req: v4l2_requestbuffers = unsafe { std::mem::zeroed() };
    req.count = count;
    req.buf_type = buf_type;
    req.memory = V4L2_MEMORY_MMAP;

    if unsafe { libc::ioctl(fd, VIDIOC_REQBUFS, &mut req) } != 0 {
        eprintln!("HwDecoder: REQBUFS failed for type {}", buf_type);
        return None;
    }

    let mut buffers = Vec::new();
    for i in 0..req.count {
        let mut plane: v4l2_plane = unsafe { std::mem::zeroed() };
        let mut buf: v4l2_buffer = unsafe { std::mem::zeroed() };
        buf.index = i;
        buf.buf_type = buf_type;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.length = 1;
        buf.m_planes = &mut plane;

        if unsafe { libc::ioctl(fd, VIDIOC_QUERYBUF, &mut buf) } != 0 {
            eprintln!("HwDecoder: QUERYBUF failed for index {}", i);
            return None;
        }

        let length = plane.length.max(min_size) as usize;
        let offset = plane.m_offset;

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                offset as libc::off_t,
            )
        };
        if ptr == libc::MAP_FAILED {
            eprintln!("HwDecoder: mmap failed for buffer {}", i);
            return None;
        }

        buffers.push(MmapBuffer { ptr: ptr as *mut u8, length });
    }

    Some(buffers)
}

fn queue_output_buf(fd: RawFd, index: u32, bytesused: u32) {
    let mut plane = v4l2_plane {
        bytesused,
        length: COMPRESSED_BUF_SIZE,
        m_offset: 0,
        _padding: 0,
        data_offset: 0,
        reserved: [0; 11],
    };
    let mut buf: v4l2_buffer = unsafe { std::mem::zeroed() };
    buf.index = index;
    buf.buf_type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    buf.memory = V4L2_MEMORY_MMAP;
    buf.length = 1;
    buf.m_planes = &mut plane;
    buf.field = V4L2_FIELD_NONE;

    unsafe { libc::ioctl(fd, VIDIOC_QBUF, &mut buf); }
}

fn queue_capture_buf(fd: RawFd, index: u32, length: usize) {
    let mut plane = v4l2_plane {
        bytesused: 0,
        length: length as u32,
        m_offset: 0,
        _padding: 0,
        data_offset: 0,
        reserved: [0; 11],
    };
    let mut buf: v4l2_buffer = unsafe { std::mem::zeroed() };
    buf.index = index;
    buf.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    buf.memory = V4L2_MEMORY_MMAP;
    buf.length = 1;
    buf.m_planes = &mut plane;

    unsafe { libc::ioctl(fd, VIDIOC_QBUF, &mut buf); }
}

fn dequeue_output_buf(fd: RawFd) -> Option<u32> {
    let mut plane: v4l2_plane = unsafe { std::mem::zeroed() };
    let mut buf: v4l2_buffer = unsafe { std::mem::zeroed() };
    buf.buf_type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    buf.memory = V4L2_MEMORY_MMAP;
    buf.length = 1;
    buf.m_planes = &mut plane;

    if unsafe { libc::ioctl(fd, VIDIOC_DQBUF, &mut buf) } == 0 {
        Some(buf.index)
    } else {
        None
    }
}

fn dequeue_capture_buf(fd: RawFd) -> Option<u32> {
    let mut plane: v4l2_plane = unsafe { std::mem::zeroed() };
    let mut buf: v4l2_buffer = unsafe { std::mem::zeroed() };
    buf.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    buf.memory = V4L2_MEMORY_MMAP;
    buf.length = 1;
    buf.m_planes = &mut plane;

    if unsafe { libc::ioctl(fd, VIDIOC_DQBUF, &mut buf) } == 0 {
        Some(buf.index)
    } else {
        None
    }
}

// ── NV12 → BGRA conversion ───────────────────────────────────────────────

fn nv12_to_bgra(
    nv12: *const u8,
    stride: u32,
    width: u32,
    height: u32,
    bgra: &mut [u8],
) {
    let stride = stride as usize;
    let w = width as usize;
    let h = height as usize;
    let y_plane = nv12;
    let uv_plane = unsafe { nv12.add(stride * h) };

    for row in 0..h {
        for col in 0..w {
            let y_idx = row * stride + col;
            let uv_idx = (row / 2) * stride + (col & !1);

            let y = unsafe { *y_plane.add(y_idx) } as f32;
            let u = unsafe { *uv_plane.add(uv_idx) } as f32 - 128.0;
            let v = unsafe { *uv_plane.add(uv_idx + 1) } as f32 - 128.0;

            // BT.709
            let r = (y + 1.5748 * v).clamp(0.0, 255.0) as u8;
            let g = (y - 0.1873 * u - 0.4681 * v).clamp(0.0, 255.0) as u8;
            let b = (y + 1.8556 * u).clamp(0.0, 255.0) as u8;

            let out_idx = (row * w + col) * 4;
            bgra[out_idx] = b;
            bgra[out_idx + 1] = g;
            bgra[out_idx + 2] = r;
            bgra[out_idx + 3] = 255;
        }
    }
}
