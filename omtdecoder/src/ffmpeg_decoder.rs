//! FFmpeg-based H.264/H.265 decoder with automatic hardware acceleration.
//! Tries platform-specific HW decoders first, falls back to software.
//!
//! macOS:   h264_videotoolbox / hevc_videotoolbox
//! Windows: h264_cuvid (NVIDIA) / h264_qsv (Intel) / h264 (sw)
//! Linux:   h264_v4l2m2m / h264 (sw)

use std::ptr;

// ── FFmpeg FFI (minimal, no bindgen) ──────────────────────────────────────

#[link(name = "avcodec")]
extern "C" {
    fn avcodec_find_decoder_by_name(name: *const i8) -> *const AVCodec;
    fn avcodec_find_decoder(id: i32) -> *const AVCodec;
    fn avcodec_alloc_context3(codec: *const AVCodec) -> *mut AVCodecContext;
    fn avcodec_free_context(ctx: *mut *mut AVCodecContext);
    fn avcodec_open2(ctx: *mut AVCodecContext, codec: *const AVCodec, options: *mut *mut AVDictionary) -> i32;
    fn avcodec_send_packet(ctx: *mut AVCodecContext, pkt: *const AVPacket) -> i32;
    fn avcodec_receive_frame(ctx: *mut AVCodecContext, frame: *mut AVFrame) -> i32;
}

#[link(name = "avutil")]
extern "C" {
    fn av_frame_alloc() -> *mut AVFrame;
    fn av_frame_free(frame: *mut *mut AVFrame);
}

#[link(name = "swscale")]
extern "C" {
    fn sws_getContext(
        srcW: i32, srcH: i32, srcFormat: i32,
        dstW: i32, dstH: i32, dstFormat: i32,
        flags: i32,
        srcFilter: *mut u8, dstFilter: *mut u8, param: *const f64,
    ) -> *mut SwsContext;
    fn sws_scale(
        ctx: *mut SwsContext,
        srcSlice: *const *const u8, srcStride: *const i32,
        srcSliceY: i32, srcSliceH: i32,
        dst: *const *mut u8, dstStride: *const i32,
    ) -> i32;
    fn sws_freeContext(ctx: *mut SwsContext);
}

// Opaque FFmpeg types
enum AVCodec {}
enum AVCodecContext {}
enum AVDictionary {}
enum SwsContext {}

#[repr(C)]
struct AVPacket {
    _buf: usize,
    pts: i64,
    dts: i64,
    data: *const u8,
    size: i32,
    _rest: [u8; 128],
}

#[repr(C)]
struct AVFrame {
    data: [*mut u8; 8],
    linesize: [i32; 8],
    _extended_data: usize,
    width: i32,
    height: i32,
    _nb_samples: i32,
    format: i32,
    _rest: [u8; 512],
}

const AV_CODEC_ID_H264: i32 = 27;
const AV_CODEC_ID_HEVC: i32 = 173;
const AV_PIX_FMT_BGRA: i32 = 28;
const SWS_BILINEAR: i32 = 2;

// ── Decoder ───────────────────────────────────────────────────────────────

pub struct FfmpegDecoder {
    ctx: *mut AVCodecContext,
    frame: *mut AVFrame,
    sws: *mut SwsContext,
    width: u32,
    height: u32,
    bgra_buf: Vec<u8>,
    sws_src_fmt: i32,
    decoder_name: String,
}

unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    pub fn new(width: u32, height: u32, codec: &str) -> Option<Self> {
        let (codec_id, hw_names) = match codec {
            "H264" => (AV_CODEC_ID_H264, h264_hw_decoders()),
            "H265" => (AV_CODEC_ID_HEVC, h265_hw_decoders()),
            _ => return None,
        };

        // Try hardware decoders first, then fall back to software
        let (av_codec, name) = find_best_decoder(codec_id, &hw_names)?;

        unsafe {
            let ctx = avcodec_alloc_context3(av_codec);
            if ctx.is_null() { return None; }

            if avcodec_open2(ctx, av_codec, ptr::null_mut()) < 0 {
                eprintln!("FfmpegDecoder: failed to open {}", name);
                avcodec_free_context(&mut (ctx as *mut _));
                return None;
            }

            let frame = av_frame_alloc();
            if frame.is_null() {
                avcodec_free_context(&mut (ctx as *mut _));
                return None;
            }

            let aligned_h = (height + 15) & !15;
            let bgra_size = (width * aligned_h * 4) as usize;

            println!("FfmpegDecoder: using {} for {} ({}x{})", name, codec, width, height);

            Some(FfmpegDecoder {
                ctx,
                frame,
                sws: ptr::null_mut(),
                width,
                height,
                bgra_buf: vec![0u8; bgra_size],
                sws_src_fmt: -1,
                decoder_name: name,
            })
        }
    }

    pub fn decode(&mut self, compressed: &[u8]) -> Option<&[u8]> {
        if compressed.is_empty() { return None; }

        unsafe {
            let mut pkt: AVPacket = std::mem::zeroed();
            pkt.data = compressed.as_ptr();
            pkt.size = compressed.len() as i32;

            if avcodec_send_packet(self.ctx, &pkt) < 0 { return None; }
            if avcodec_receive_frame(self.ctx, self.frame) < 0 { return None; }

            let f = &*self.frame;
            let src_fmt = f.format;
            let src_h = f.height;

            // Reinit sws if source format changed
            if src_fmt != self.sws_src_fmt {
                if !self.sws.is_null() { sws_freeContext(self.sws); }
                self.sws = sws_getContext(
                    f.width, f.height, src_fmt,
                    self.width as i32, self.height as i32, AV_PIX_FMT_BGRA,
                    SWS_BILINEAR,
                    ptr::null_mut(), ptr::null_mut(), ptr::null(),
                );
                if self.sws.is_null() {
                    eprintln!("FfmpegDecoder: sws_getContext failed (fmt={})", src_fmt);
                    return None;
                }
                self.sws_src_fmt = src_fmt;
            }

            let dst_stride = (self.width * 4) as i32;
            let dst_ptr = self.bgra_buf.as_mut_ptr();
            let dst_ptrs = [dst_ptr];
            let dst_strides = [dst_stride];

            sws_scale(
                self.sws,
                f.data.as_ptr() as *const *const u8,
                f.linesize.as_ptr(),
                0, src_h,
                dst_ptrs.as_ptr() as *const *mut u8,
                dst_strides.as_ptr(),
            );

            Some(&self.bgra_buf[..self.width as usize * self.height as usize * 4])
        }
    }

    pub fn name(&self) -> &str {
        &self.decoder_name
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        unsafe {
            if !self.sws.is_null() { sws_freeContext(self.sws); }
            if !self.frame.is_null() { av_frame_free(&mut self.frame); }
            if !self.ctx.is_null() { avcodec_free_context(&mut self.ctx); }
        }
    }
}

// ── Platform-specific HW decoder lists ────────────────────────────────────

fn h264_hw_decoders() -> Vec<&'static str> {
    let mut v = Vec::new();
    #[cfg(target_os = "macos")]   v.push("h264_videotoolbox");
    #[cfg(target_os = "windows")] { v.push("h264_cuvid"); v.push("h264_qsv"); }
    #[cfg(target_os = "linux")]   v.push("h264_v4l2m2m");
    v
}

fn h265_hw_decoders() -> Vec<&'static str> {
    let mut v = Vec::new();
    #[cfg(target_os = "macos")]   v.push("hevc_videotoolbox");
    #[cfg(target_os = "windows")] { v.push("hevc_cuvid"); v.push("hevc_qsv"); }
    #[cfg(target_os = "linux")]   v.push("hevc_v4l2m2m");
    v
}

fn find_best_decoder(codec_id: i32, hw_names: &[&str]) -> Option<(*const AVCodec, String)> {
    // Try HW decoders first
    for name in hw_names {
        let cname = format!("{}\0", name);
        let codec = unsafe { avcodec_find_decoder_by_name(cname.as_ptr() as *const i8) };
        if !codec.is_null() {
            return Some((codec, name.to_string()));
        }
    }

    // Fall back to generic software decoder
    let codec = unsafe { avcodec_find_decoder(codec_id) };
    if !codec.is_null() {
        let name = match codec_id {
            AV_CODEC_ID_H264 => "h264 (software)",
            AV_CODEC_ID_HEVC => "hevc (software)",
            _ => "unknown",
        };
        return Some((codec, name.to_string()));
    }

    eprintln!("FfmpegDecoder: no decoder found for codec_id={}", codec_id);
    None
}
