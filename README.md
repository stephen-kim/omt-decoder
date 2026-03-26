# OMT Decoder

한국어 문서는 [README.ko.md](README.ko.md)를 참고하세요.

A low-latency OMT stream receiver and player for Raspberry Pi, rewritten in **Rust** for maximum performance.

## Why Rust?

The original C#/.NET implementation required a 200MB+ .NET runtime and suffered from GC pauses and managed-to-native marshalling overhead. The Rust port eliminates all of that:

| | C# (.NET 8 AOT) | Rust |
|---|---|---|
| **Runtime dependency** | .NET 8 (~200MB) | None (static binary) |
| **Binary size** | ~30MB + runtime | ~5MB standalone |
| **VMX decode** | Managed array + P/Invoke marshal | Zero-copy, pre-allocated buffer |
| **Audio path** | GC-managed buffers, pool allocation | Direct ALSA FFI, no allocator pressure |
| **Frame delivery** | Async socket + frame pool + GC | Blocking TCP read, zero-copy parse |
| **Memory** | ~150MB RSS | ~70MB RSS |
| **Video latency** | ~20-30ms | ~18-30ms (decode 7-12ms + vsync) |

### Architecture

```
Player thread:  TCP read → audio writei → video channel
Video thread:   channel recv → VMX decode → DRM page-flip
Tokio runtime:  Web server + mDNS discovery (non-interfering)
```

- **No GC, no runtime** — deterministic latency, no pause spikes
- **Zero-copy frame parsing** — `BytesMut` splits directly from TCP buffer
- **Pre-allocated decode buffer** — VMX decodes into a reused buffer, no per-frame allocation
- **Direct ALSA FFI** — `snd_pcm_writei` called from the TCP read thread, matching C#'s P/Invoke path exactly
- **DRM page-flip** — triple-buffered vsync output with dedicated event thread

### Web UI

Built-in web control panel with dark theme (matching omt-encoder style):
- Source selection with mDNS auto-discovery
- Audio output device selection
- Volume slider (0-200%)
- Multi-language support (English, Korean, Japanese, Spanish)
- Toast notifications

## Install on Raspberry Pi

### 1. Set Console Boot

`omtplayer` outputs directly via DRM — desktop mode must be disabled.

```bash
sudo raspi-config
```

Choose `1 System Options` -> `S5 Boot` -> `B1 Console Text console`.

### 2. Clone and Build

```bash
git clone https://github.com/stephen-kim/omt-decoder.git
cd omt-decoder
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

The script installs Rust toolchain + dependencies, builds the project, deploys to `/opt/omtplayer`, and registers the systemd service.

### 3. Check Status

```bash
sudo systemctl status omtplayer
```

Web UI: `http://<pi-ip>:8080/`

## License

MIT License. See [LICENSE](LICENSE) for details.
