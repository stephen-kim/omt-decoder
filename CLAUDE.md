# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is this?

Low-latency OMT (Open Media Transport) stream receiver/player for Raspberry Pi, written in Rust. Receives VMX1/H.264/H.265 video and FPA1 audio over TCP, decodes, and outputs via DRM (video) + ALSA (audio). Includes a web control panel.

Ported from C# (.NET 8 AOT). When architecture decisions are unclear, match the original C# structure — it's battle-tested.

## Build

```bash
cargo check                                    # macOS (Linux-only code is cfg-gated)
cargo build --release                          # full build (with FFmpeg)
cargo build --release --no-default-features    # without FFmpeg
```

On Raspberry Pi (first time):
```bash
./build_and_install_service.sh    # installs Rust, deps, builds, deploys to /opt/omtdecoder
```

## Deploy to Pi

```bash
ssh -i ~/.ssh/omt-decode-2 cpm@192.168.1.112
cd ~/omt-decoder && git pull --recurse-submodules && source ~/.cargo/env
cargo build --release
sudo systemctl stop omtdecoder && sudo cp target/release/omtdecoder /opt/omtdecoder/ && sudo systemctl start omtdecoder
sudo journalctl -u omtdecoder -f   # check logs
```

## Architecture

Three threads, deliberately simple (matching C# original):

```
Player thread:   blocking TCP read → parse OMT frames
                 → Audio frames: snd_pcm_writei directly (no queue, no audio thread)
                 → Video frames: send to video thread via sync_channel(2)

Video thread:    recv from channel → VMX decode → DRM page-flip (triple-buffered)

Tokio runtime:   Web server (Axum, port 8080) + mDNS discovery (avahi-browse async)
                  These never touch the frame path.
```

The player thread does blocking TCP I/O. Tokio is only for the web server and discovery — it must never be on the critical frame delivery path (caused 50% frame loss when we tried).

## Submodules

```
libomtnet/   → github.com/stephen-kim/libomtnet-rs   (OMT network protocol, shared with omt-encoder and omt-switcher)
libvmx/      → github.com/stephen-kim/libvmx          (VMX video codec C++, fork with clang patches)
libvmx-sys/  → local FFI bindings (build.rs compiles libvmx C++ and generates Rust bindings via bindgen)
```

After cloning: `git submodule update --init --recursive`

## Critical rules (hard-won from SEGV debugging)

**VMX decoder:**
- `VMX_LoadFrom` takes non-const `BYTE*` and **writes to its input buffer**. Always copy compressed data to a mutable `Vec` before calling. Passing `Bytes` (read-only refcounted) causes SEGV.
- Decoder profile must be `VMX_PROFILE_DEFAULT` (0). Using encoder profiles like `OMT_HQ` (199) causes internal buffer overflow → SEGV after a few seconds.
- Decode buffer height must be aligned to 16-row slices: `(height + 15) & !15`. For 1080p this means 1088 rows. Without this, the decoder writes past the buffer end → SEGV.

**DRM presenter:**
- `front_buffer` must start as `None` (matching C# `null`). Setting `Some(first_idx)` causes the same buffer to exist in both `front_buffer` and `write_queue` simultaneously → visible frame corruption/tearing.

**Audio:**
- Use `SND_PCM_FORMAT_S16_LE` (not float). USB DACs don't handle float format reliably through `plughw`.
- Write directly from the player thread via `snd_pcm_writei` FFI. A separate audio thread with condvar/queue caused periodic underruns.
- ALSA latency: 50ms (matching C# `snd_pcm_set_params` with latency=50000).

**OMT protocol:**
- Client must send `<OMTSubscribe Video="true" />` etc. after connecting, otherwise the server only sends metadata frames.
- avahi-browse must run via `tokio::process::Command` (async), not `std::process::Command` (blocks tokio worker → frame stalls).
- avahi-browse output escapes special chars as `\DDD` (decimal) and `\.` — must be unescaped for display.

## Codec negotiation

The decoder sends `<OMTSettings Quality="..." Codec="H265,H264,VMX1" />` to request preferred codec. The encoder picks the best match it supports. Video frames arrive with `video_header.codec` indicating the actual codec used. The video thread routes to the appropriate decoder:

- VMX1 → `vmx_decoder.rs` (libvmx software decode)
- H.264/H.265 → `hw_decoder.rs` (V4L2 M2M) or `ffmpeg_decoder.rs` (FFmpeg, feature-gated)
- BGRA → passthrough

## Related repositories

- `stephen-kim/omt-encoder` — Rust, Raspberry Pi capture/encode
- `stephen-kim/omt-switcher` — Rust + Tauri, desktop video switcher
- `stephen-kim/libomtnet-rs` — shared OMT protocol (submodule in all three)
- `stephen-kim/libvmx` — shared VMX codec (submodule in all three)
