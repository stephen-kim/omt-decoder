use libc::{c_int, c_ulong, c_void};
use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::ptr;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

// ── DRM constants ──────────────────────────────────────────────────────────

const DRM_CAP_DUMB_BUFFER: u64 = 1;
const DRM_FORMAT_XRGB8888: u32 = 0x34325258;
const DRM_MODE_PAGE_FLIP_EVENT: u32 = 1;
const DRM_MODE_FLAG_INTERLACE: u32 = 1 << 4;
const DRM_EVENT_CONTEXT_VERSION: c_int = 4;
const O_RDWR: c_int = 2;
const POLLIN: i16 = 0x001;

// ── ioctl codes (Linux) ────────────────────────────────────────────────────

const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> c_ulong {
    ((dir << 30) | (ty << 8) | nr | (size << 16)) as c_ulong
}
const IOC_RDWR: u32 = 3;
const DRM_IOCTL_MODE_CREATE_DUMB: c_ulong = ioc(IOC_RDWR, b'd' as u32, 0xB2, 32);
const DRM_IOCTL_MODE_MAP_DUMB: c_ulong = ioc(IOC_RDWR, b'd' as u32, 0xB3, 16);
const DRM_IOCTL_MODE_DESTROY_DUMB: c_ulong = ioc(IOC_RDWR, b'd' as u32, 0xB4, 4);

// ── DRM FFI structures ────────────────────────────────────────────────────

#[repr(C)]
struct DrmModeRes {
    count_fbs: c_int,
    fbs: *mut u32,
    count_crtcs: c_int,
    crtcs: *mut u32,
    count_connectors: c_int,
    connectors: *mut u32,
    count_encoders: c_int,
    encoders: *mut u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
struct DrmModeConnector {
    connector_id: u32,
    encoder_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    count_modes: c_int,
    modes: *mut DrmModeModeInfo,
    count_props: c_int,
    props: *mut u32,
    prop_values: *mut u64,
    count_encoders: c_int,
    encoders: *mut u32,
}

#[repr(C)]
struct DrmModeEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrmModeModeInfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub vrefresh: u32,
    pub flags: u32,
    pub type_: u32,
    pub name: [u8; 32],
}

#[repr(C)]
struct DrmModeCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
struct DrmModeMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
struct DrmModeDestroyDumb {
    handle: u32,
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

type PageFlipHandler = unsafe extern "C" fn(c_int, u32, u32, u32, *mut c_void);

#[repr(C)]
struct DrmEventContext {
    version: c_int,
    vblank_handler: *const c_void,
    page_flip_handler: Option<PageFlipHandler>,
    page_flip_handler2: *const c_void,
    sequence_handler: *const c_void,
}

// ── DRM FFI functions ──────────────────────────────────────────────────────

extern "C" {
    fn open(path: *const u8, flags: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

#[link(name = "drm")]
extern "C" {
    fn drmGetCap(fd: c_int, capability: u64, value: *mut u64) -> c_int;
    fn drmModeGetResources(fd: c_int) -> *mut DrmModeRes;
    fn drmModeFreeResources(ptr: *mut DrmModeRes);
    fn drmModeGetConnector(fd: c_int, connector_id: u32) -> *mut DrmModeConnector;
    fn drmModeFreeConnector(ptr: *mut DrmModeConnector);
    fn drmModeGetEncoder(fd: c_int, encoder_id: u32) -> *mut DrmModeEncoder;
    fn drmModeFreeEncoder(ptr: *mut DrmModeEncoder);
    fn drmModeAddFB2(
        fd: c_int,
        width: u32,
        height: u32,
        pixel_format: u32,
        bo_handles: *const u32,
        pitches: *const u32,
        offsets: *const u32,
        buf_id: *mut u32,
        flags: u32,
    ) -> c_int;
    fn drmModeRmFB(fd: c_int, buffer_id: u32) -> c_int;
    fn drmModePageFlip(
        fd: c_int,
        crtc_id: u32,
        fb_id: u32,
        flags: u32,
        user_data: *mut c_void,
    ) -> c_int;
    fn drmModeSetCrtc(
        fd: c_int,
        crtc_id: u32,
        fb_id: u32,
        x: u32,
        y: u32,
        connectors: *const u32,
        count: c_int,
        mode: *mut DrmModeModeInfo,
    ) -> c_int;
    fn drmHandleEvent(fd: c_int, evctx: *mut DrmEventContext) -> c_int;
}

// ── High-level types ───────────────────────────────────────────────────────

struct DrmBuffer {
    fd: RawFd,
    fb_id: u32,
    handle: u32,
    pitch: u32,
    size: u64,
    width: u32,
    height: u32,
    mapping: *mut u8,
}

impl DrmBuffer {
    fn new(fd: RawFd, width: u32, height: u32) -> Option<Self> {
        let mut create = DrmModeCreateDumb {
            height,
            width,
            bpp: 32,
            flags: 0,
            handle: 0,
            pitch: 0,
            size: 0,
        };
        let hr = unsafe { ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &mut create as *mut _) };
        if hr != 0 {
            return None;
        }

        let handles = [create.handle, 0, 0, 0];
        let pitches = [create.pitch, 0, 0, 0];
        let offsets = [0u32; 4];
        let mut fb_id: u32 = 0;

        let hr = unsafe {
            drmModeAddFB2(
                fd,
                width,
                height,
                DRM_FORMAT_XRGB8888,
                handles.as_ptr(),
                pitches.as_ptr(),
                offsets.as_ptr(),
                &mut fb_id,
                0,
            )
        };
        if hr != 0 {
            return None;
        }

        Some(DrmBuffer {
            fd,
            fb_id,
            handle: create.handle,
            pitch: create.pitch,
            size: create.size,
            width,
            height,
            mapping: ptr::null_mut(),
        })
    }

    fn map(&mut self) -> *mut u8 {
        if self.mapping.is_null() {
            let mut map_req = DrmModeMapDumb {
                handle: self.handle,
                pad: 0,
                offset: 0,
            };
            let hr = unsafe { ioctl(self.fd, DRM_IOCTL_MODE_MAP_DUMB, &mut map_req as *mut _) };
            if hr != 0 {
                return ptr::null_mut();
            }
            let ptr = unsafe {
                mmap(
                    ptr::null_mut(),
                    self.size as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    self.fd,
                    map_req.offset as i64,
                )
            };
            if ptr == libc::MAP_FAILED {
                return ptr::null_mut();
            }
            self.mapping = ptr as *mut u8;
        }
        self.mapping
    }

    fn copy_from(&mut self, src: &[u8], src_stride: u32) {
        let dst = self.map();
        if dst.is_null() {
            return;
        }
        let row_bytes = (self.width * 4) as usize;
        let src_stride = src_stride as usize;
        // Bounds check: ensure src is large enough
        let rows = self.height as usize;
        let required = if rows > 0 { (rows - 1) * src_stride + row_bytes } else { 0 };
        if src.len() < required {
            return; // source buffer too small, skip frame
        }
        unsafe {
            for y in 0..rows {
                let dst_row = dst.add(y * self.pitch as usize);
                let src_row = src.as_ptr().add(y * src_stride);
                ptr::copy_nonoverlapping(src_row, dst_row, row_bytes);
            }
        }
    }
}

impl Drop for DrmBuffer {
    fn drop(&mut self) {
        if !self.mapping.is_null() {
            unsafe {
                munmap(self.mapping as *mut c_void, self.size as usize);
            }
            self.mapping = ptr::null_mut();
        }
        if self.fb_id != 0 {
            unsafe {
                drmModeRmFB(self.fd, self.fb_id);
            }
        }
        if self.handle != 0 {
            let mut destroy = DrmModeDestroyDumb {
                handle: self.handle,
            };
            unsafe {
                ioctl(self.fd, DRM_IOCTL_MODE_DESTROY_DUMB, &mut destroy as *mut _);
            }
        }
    }
}

// ── Video Output (DRM presenter) ──────────────────────────────────────────

pub struct VideoOutput {
    fd: RawFd,
    crtc_id: u32,
    buffers: Vec<DrmBuffer>,
    write_queue: VecDeque<usize>,
    present_queue: VecDeque<usize>,
    front_buffer: Option<usize>,
    present_empty: bool,
    events_running: Arc<Mutex<bool>>,
    events_thread: Option<std::thread::JoinHandle<()>>,
    flip_rx: std_mpsc::Receiver<u32>,
}

impl VideoOutput {
    pub fn new(width: u32, height: u32, frame_rate: f32) -> Option<Self> {
        // Try card0, card1, card2
        let fd = try_open_drm_device();
        if fd < 0 {
            eprintln!("VideoOutput: no usable DRM device found");
            return None;
        }

        // Find connector and mode
        let found = find_connector_and_mode(fd, width, height, frame_rate);
        if found.is_none() {
            eprintln!("VideoOutput: no matching display mode for {}x{} @ {:.1}fps", width, height, frame_rate);
            unsafe { close(fd); }
            return None;
        }
        let (connector_id, encoder_crtc_id, mode) = found.unwrap();

        // Create triple-buffered framebuffers
        let mode_w = mode.hdisplay as u32;
        let mode_h = mode.vdisplay as u32;
        println!("VideoOutput: mode {}x{} @ {}Hz", mode_w, mode_h, mode.vrefresh);
        let mut buffers = Vec::new();
        let mut write_queue = VecDeque::new();
        for i in 0..3 {
            let buf = DrmBuffer::new(fd, mode_w, mode_h);
            if buf.is_none() {
                eprintln!("VideoOutput: failed to create DRM buffer {}", i);
                unsafe { close(fd); }
                return None;
            }
            buffers.push(buf.unwrap());
            write_queue.push_back(i);
        }

        // Set initial CRTC
        let first_idx = write_queue.pop_front().unwrap();
        let connectors = [connector_id];
        let mut mode_copy = mode;
        let hr = unsafe {
            drmModeSetCrtc(
                fd,
                encoder_crtc_id,
                buffers[first_idx].fb_id,
                0,
                0,
                connectors.as_ptr(),
                1,
                &mut mode_copy,
            )
        };
        if hr != 0 {
            eprintln!("drmModeSetCrtc failed: {}", hr);
            unsafe { close(fd); }
            return None;
        }

        let events_running = Arc::new(Mutex::new(true));
        let (flip_tx, flip_rx) = std_mpsc::sync_channel::<u32>(4);

        // Start events thread
        let fd_clone = fd;
        let running_clone = events_running.clone();
        let events_handle = std::thread::Builder::new()
            .name("drm-events".into())
            .spawn(move || {
                events_thread(fd_clone, running_clone, flip_tx);
            })
            .expect("failed to spawn DRM events thread");

        // Do initial flip
        let fb_id = buffers[first_idx].fb_id;
        unsafe {
            drmModePageFlip(
                fd,
                encoder_crtc_id,
                fb_id,
                DRM_MODE_PAGE_FLIP_EVENT,
                fb_id as *mut c_void,
            );
        }

        let vo = VideoOutput {
            fd,
            crtc_id: encoder_crtc_id,
            buffers,
            write_queue,
            present_queue: VecDeque::new(),
            front_buffer: None, // C# starts with null — set by first FlippedEvent
            present_empty: false,
            events_running,
            events_thread: Some(events_handle),
            flip_rx,
        };

        Some(vo)
    }

    pub fn present(&mut self, bgra_data: &[u8], stride: u32) {
        // Check for completed flips
        self.process_flip_events();

        if let Some(idx) = self.write_queue.pop_front() {
            self.buffers[idx].copy_from(bgra_data, stride);

            if self.present_empty {
                self.present_empty = false;
                self.flip(idx);
            } else {
                // Only keep the latest frame queued — drop stale ones to prevent
                // uneven frame pacing when source fps < display refresh rate.
                while let Some(old) = self.present_queue.pop_front() {
                    self.write_queue.push_back(old);
                }
                self.present_queue.push_back(idx);
            }
        }
    }

    fn process_flip_events(&mut self) {
        while let Ok(fb_id) = self.flip_rx.try_recv() {
            // Return front buffer to write queue
            if let Some(front) = self.front_buffer.take() {
                self.write_queue.push_back(front);
            }
            // Find which buffer was flipped
            for (i, buf) in self.buffers.iter().enumerate() {
                if buf.fb_id == fb_id {
                    self.front_buffer = Some(i);
                    break;
                }
            }
            // Present next queued buffer
            if let Some(next) = self.present_queue.pop_front() {
                self.flip(next);
            } else {
                self.present_empty = true;
            }
        }
    }

    fn flip(&self, buf_idx: usize) {
        let fb_id = self.buffers[buf_idx].fb_id;
        unsafe {
            drmModePageFlip(
                self.fd,
                self.crtc_id,
                fb_id,
                DRM_MODE_PAGE_FLIP_EVENT,
                fb_id as *mut c_void,
            );
        }
    }
}

impl Drop for VideoOutput {
    fn drop(&mut self) {
        *self.events_running.lock().unwrap() = false;
        // Wait for events thread to finish (it polls with 200ms timeout)
        if let Some(handle) = self.events_thread.take() {
            let _ = handle.join();
        }
        self.buffers.clear();
        if self.fd >= 0 {
            unsafe {
                close(self.fd);
            }
        }
    }
}

/// We need to pass the SyncSender into the C callback. Since drmHandleEvent
/// calls the callback synchronously on the same thread, a thread-local is safe.
thread_local! {
    static FLIP_TX: std::cell::RefCell<Option<std_mpsc::SyncSender<u32>>> = std::cell::RefCell::new(None);
}

unsafe extern "C" fn flip_handler(
    _fd: c_int,
    _seq: u32,
    _tv_sec: u32,
    _tv_usec: u32,
    user_data: *mut c_void,
) {
    let fb_id = user_data as u32;
    FLIP_TX.with(|cell| {
        if let Some(ref tx) = *cell.borrow() {
            let _ = tx.try_send(fb_id);
        }
    });
}

fn events_thread(
    fd: RawFd,
    running: Arc<Mutex<bool>>,
    flip_tx: std_mpsc::SyncSender<u32>,
) {
    // Store sender in thread-local so the C callback can use it
    FLIP_TX.with(|cell| {
        *cell.borrow_mut() = Some(flip_tx);
    });

    let mut ctx = DrmEventContext {
        version: DRM_EVENT_CONTEXT_VERSION,
        vblank_handler: ptr::null(),
        page_flip_handler: Some(flip_handler),
        page_flip_handler2: ptr::null(),
        sequence_handler: ptr::null(),
    };

    let mut pfd = PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    };

    loop {
        if !*running.lock().unwrap() {
            break;
        }

        let hr = unsafe { poll(&mut pfd as *mut _, 1, 200) };
        if hr <= 0 {
            continue;
        }

        unsafe {
            drmHandleEvent(fd, &mut ctx);
        }
    }
}

fn try_open_drm_device() -> RawFd {
    for i in 0..4 {
        let path = format!("/dev/dri/card{}\0", i);
        let fd = unsafe { open(path.as_ptr(), O_RDWR) };
        if fd < 0 {
            continue;
        }
        let mut cap: u64 = 0;
        if unsafe { drmGetCap(fd, DRM_CAP_DUMB_BUFFER, &mut cap) } != 0 || cap == 0 {
            eprintln!("VideoOutput: /dev/dri/card{} no dumb buffer support, skipping", i);
            unsafe { close(fd); }
            continue;
        }
        println!("VideoOutput: opened /dev/dri/card{}", i);
        return fd;
    }
    -1
}

fn find_connector_and_mode(
    fd: RawFd,
    width: u32,
    height: u32,
    frame_rate: f32,
) -> Option<(u32, u32, DrmModeModeInfo)> {
    unsafe {
        let res = drmModeGetResources(fd);
        if res.is_null() {
            return None;
        }

        for i in 0..(*res).count_connectors {
            let conn_id = *(*res).connectors.add(i as usize);
            let conn = drmModeGetConnector(fd, conn_id);
            if conn.is_null() {
                continue;
            }

            // Check if connected
            if (*conn).connection != 1 {
                drmModeFreeConnector(conn);
                continue;
            }

            // Get encoder → CRTC
            let enc = drmModeGetEncoder(fd, (*conn).encoder_id);
            if enc.is_null() {
                drmModeFreeConnector(conn);
                continue;
            }
            let crtc_id = (*enc).crtc_id;
            drmModeFreeEncoder(enc);

            // Find best mode
            let modes =
                std::slice::from_raw_parts((*conn).modes, (*conn).count_modes as usize);
            let mode = find_nearest_mode(modes, width, height, frame_rate);

            let connector_id = (*conn).connector_id;
            drmModeFreeConnector(conn);
            drmModeFreeResources(res);

            if let Some(m) = mode {
                return Some((connector_id, crtc_id, m));
            }
            return None;
        }

        drmModeFreeResources(res);
        None
    }
}

fn find_nearest_mode(
    modes: &[DrmModeModeInfo],
    width: u32,
    height: u32,
    frame_rate: f32,
) -> Option<DrmModeModeInfo> {
    let target_rate = (frame_rate * 100.0).round() as u32;
    let target_rate_rounded = (frame_rate.round() * 100.0) as u32;

    fn mode_rate(m: &DrmModeModeInfo) -> u32 {
        let rate =
            (m.clock as f64 * 1000.0 / (m.htotal as f64 * m.vtotal as f64) * 100.0).round()
                as u32;
        rate
    }

    // Exact match
    for m in modes {
        if m.hdisplay as u32 == width
            && m.vdisplay as u32 == height
            && (m.flags & DRM_MODE_FLAG_INTERLACE) == 0
        {
            let rate = mode_rate(m);
            if rate == target_rate {
                return Some(*m);
            }
        }
    }

    // Rounded match
    for m in modes {
        if m.hdisplay as u32 == width
            && m.vdisplay as u32 == height
            && (m.flags & DRM_MODE_FLAG_INTERLACE) == 0
        {
            let rate = mode_rate(m);
            if rate == target_rate_rounded {
                return Some(*m);
            }
        }
    }

    // 60fps fallback
    for m in modes {
        if m.hdisplay as u32 == width
            && m.vdisplay as u32 == height
            && (m.flags & DRM_MODE_FLAG_INTERLACE) == 0
        {
            let rate = mode_rate(m);
            if rate == 6000 {
                return Some(*m);
            }
        }
    }

    None
}
