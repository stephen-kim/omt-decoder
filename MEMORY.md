# Memory — omt-decoder

## Architecture
- Single player thread: blocking TCP read → audio writei → video to channel
- Separate video thread: VMX decode → DRM page-flip
- Tokio only for web server + mDNS discovery (non-interfering)
- Audio: direct snd_pcm_writei FFI from player thread, no separate audio thread

## Critical bugs & fixes
- **VMX_LoadFrom**: Writes to input buffer. Always copy compressed data to mutable Vec first (was causing SEGV from writing to read-only Bytes).
- **VMX profile**: Decoder must use VMX_PROFILE_DEFAULT (0). Using encoder profiles (OMT_HQ=199) causes buffer overflow → SEGV.
- **VMX buffer alignment**: Height must be `(h + 15) & !15`. For 1080p → 1088 rows. Without this, decoder writes past buffer end → SEGV.
- **DRM front_buffer**: Must start as None. Setting Some(first_idx) causes same buffer in both front_buffer and write_queue → frame corruption.
- **Audio format**: Use S16_LE (not float). USB DACs don't handle float well through plughw.
- **Audio latency**: 50ms ALSA buffer (matching C# original). Audio enqueued directly from player thread.
- **std::sync::mpsc**: Caused 50% frame loss for video. Use direct blocking TCP read instead.
- **avahi-browse**: Must use tokio::process::Command (not std::process) to avoid blocking tokio workers. Escape \DDD sequences and \. in service names.

## Key relationships
- Ported from C# (.NET 8 AOT). When in doubt, match C# structure exactly.
- Pi SSH: `ssh -i ~/.ssh/omt-decode-2 cpm@192.168.1.112`
- Service name: omtdecoder (was omtplayer)
- Install dir: /opt/omtdecoder
