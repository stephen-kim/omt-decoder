# CLAUDE.md — omt-decoder

## What is this?
Rust-based OMT stream receiver/player for Raspberry Pi. Receives video (VMX1/H.264/H.265) and audio (FPA1) over TCP, decodes, and outputs via DRM (video) + ALSA (audio). Web UI for control.

## Build
```bash
cargo check                          # macOS (Linux-only code is cfg-gated)
cargo build --release                # full build
cargo build --release --no-default-features  # without FFmpeg
```

## Architecture
```
Player thread:  TCP read → audio writei → video to channel
Video thread:   VMX decode → DRM page-flip
Tokio runtime:  Web server (Axum) + mDNS discovery (non-interfering)
```

## Key files
- `omtdecoder/src/main.rs` — main loop, player_loop(), video_thread()
- `omtdecoder/src/receiver.rs` — OMTConnection: blocking TCP, frame parsing, subscriptions
- `omtdecoder/src/vmx_decoder.rs` — VMX1 software decode (libvmx FFI)
- `omtdecoder/src/audio.rs` — ALSA playback (direct snd_pcm_writei FFI)
- `omtdecoder/src/video.rs` — DRM output (dumb buffers, page-flip, events thread)
- `omtdecoder/src/hw_decoder.rs` — V4L2 M2M hardware decode (H.264/H.265)
- `omtdecoder/src/ffmpeg_decoder.rs` — FFmpeg cross-platform decode (feature-gated)
- `omtdecoder/src/web_server.rs` — Axum REST API
- `omtdecoder/src/index.html` — Web UI (dark theme, i18n)
- `omtdecoder/src/settings.rs` — JSON config
- `omtdecoder/src/discovery.rs` — avahi-browse mDNS

## Submodules
- `libomtnet/` → github.com/stephen-kim/libomtnet-rs
- `libvmx/` → github.com/stephen-kim/libvmx

## Critical rules
- VMX_LoadFrom writes to input buffer → always copy to mutable Vec first
- VMX decoder profile must be DEFAULT (0), not encoder profiles
- VMX decode buffer: align height to 16 rows `(h + 15) & !15`
- DRM front_buffer starts as None (not Some)
- Audio: direct snd_pcm_writei from player thread, no separate audio thread
- Match C# structure when in doubt — it's battle-tested

## Deploy to Pi
```bash
ssh -i ~/.ssh/omt-decode-2 cpm@192.168.1.112
cd ~/omt-decoder && git pull && source ~/.cargo/env
cargo build --release
sudo systemctl stop omtdecoder
sudo cp target/release/omtdecoder /opt/omtdecoder/
sudo systemctl start omtdecoder
```
