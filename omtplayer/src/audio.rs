use std::ffi::CString;
use std::sync::Mutex;

// ── ALSA FFI ──────────────────────────────────────────────────────────────

#[link(name = "asound")]
extern "C" {}

const SND_PCM_STREAM_PLAYBACK: libc::c_int = 0;
const SND_PCM_FORMAT_S16_LE: libc::c_int = 2;
const SND_PCM_ACCESS_RW_INTERLEAVED: libc::c_int = 3;

extern "C" {
    fn snd_pcm_open(
        pcm: *mut *mut libc::c_void,
        name: *const libc::c_char,
        stream: libc::c_int,
        mode: libc::c_int,
    ) -> libc::c_int;
    fn snd_pcm_set_params(
        pcm: *mut libc::c_void,
        format: libc::c_int,
        access: libc::c_int,
        channels: libc::c_uint,
        rate: libc::c_uint,
        soft_resample: libc::c_int,
        latency: libc::c_uint,
    ) -> libc::c_int;
    fn snd_pcm_writei(
        pcm: *mut libc::c_void,
        buffer: *const libc::c_void,
        size: libc::c_ulong,
    ) -> libc::c_long;
    fn snd_pcm_recover(
        pcm: *mut libc::c_void,
        err: libc::c_int,
        silent: libc::c_int,
    ) -> libc::c_int;
    fn snd_pcm_close(pcm: *mut libc::c_void) -> libc::c_int;
    fn snd_strerror(errnum: libc::c_int) -> *const libc::c_char;
}

fn alsa_error(err: libc::c_int) -> String {
    unsafe {
        let ptr = snd_strerror(err);
        if ptr.is_null() {
            format!("ALSA error {}", err)
        } else {
            std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string()
        }
    }
}

// ── PCM Handle ────────────────────────────────────────────────────────────

struct PcmHandle {
    handle: *mut libc::c_void,
    name: String,
}

unsafe impl Send for PcmHandle {}

impl Drop for PcmHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { snd_pcm_close(self.handle); }
            self.handle = std::ptr::null_mut();
        }
    }
}

// ── Audio Player ──────────────────────────────────────────────────────────
// No separate thread — writes directly to ALSA from enqueue().
// This eliminates queue/condvar latency that caused underruns.

pub struct AudioPlayer {
    device_names: Vec<String>,
    handles: Vec<PcmHandle>,
    channels: u32,
    rate: u32,
    volume: f32,
}

impl AudioPlayer {
    pub fn new(device_names: &[String], volume: f32) -> Self {
        AudioPlayer {
            device_names: device_names.to_vec(),
            handles: Vec::new(),
            channels: 0,
            rate: 0,
            volume,
        }
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 2.0);
    }

    pub fn set_devices(&mut self, device_names: &[String]) {
        self.device_names = device_names.to_vec();
        self.handles.clear();
        self.channels = 0;
    }

    pub fn enqueue(
        &mut self,
        planar_data: &[u8],
        channels: u32,
        samples_per_channel: u32,
        sample_rate: u32,
    ) {
        // Re-open if format changed or not yet opened
        if channels != self.channels || sample_rate != self.rate || self.handles.is_empty() {
            self.open_audio(channels, sample_rate);
        }

        if self.handles.is_empty() {
            return;
        }

        let total_samples = (channels * samples_per_channel) as usize;
        let required_bytes = total_samples * std::mem::size_of::<f32>();
        if planar_data.len() < required_bytes {
            return;
        }

        // Planar float → interleaved S16 with volume
        let mut interleaved = vec![0i16; total_samples];
        let vol = self.volume;

        let src = unsafe {
            std::slice::from_raw_parts(planar_data.as_ptr() as *const f32, total_samples)
        };

        for s in 0..samples_per_channel as usize {
            for c in 0..channels as usize {
                let sample = src[c * samples_per_channel as usize + s] * vol;
                interleaved[s * channels as usize + c] = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            }
        }

        // Write directly to all ALSA devices
        let frames = samples_per_channel as libc::c_ulong;
        for dev in &self.handles {
            let err = unsafe {
                snd_pcm_writei(dev.handle, interleaved.as_ptr() as *const libc::c_void, frames)
            };
            if err < 0 {
                unsafe { snd_pcm_recover(dev.handle, err as libc::c_int, 1); }
            }
        }
    }

    fn open_audio(&mut self, channels: u32, sample_rate: u32) {
        self.handles.clear();

        for name in &self.device_names {
            match open_pcm(name, channels, sample_rate) {
                Ok(handle) => {
                    println!("Audio opened ({}): {}ch {}Hz", name, channels, sample_rate);
                    self.handles.push(handle);
                }
                Err(e) => {
                    eprintln!("Audio open error ({}): {}", name, e);
                }
            }
        }

        self.channels = channels;
        self.rate = sample_rate;
    }
}

fn open_pcm(name: &str, channels: u32, sample_rate: u32) -> Result<PcmHandle, String> {
    let cname = CString::new(name).map_err(|e| e.to_string())?;
    let mut handle: *mut libc::c_void = std::ptr::null_mut();

    let err = unsafe {
        snd_pcm_open(&mut handle, cname.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0)
    };
    if err < 0 {
        return Err(format!("snd_pcm_open: {}", alsa_error(err)));
    }

    // Match C#: snd_pcm_set_params(handle, format, access, ch, rate, soft_resample, latency_us)
    let err = unsafe {
        snd_pcm_set_params(
            handle,
            SND_PCM_FORMAT_S16_LE,
            SND_PCM_ACCESS_RW_INTERLEAVED,
            channels,
            sample_rate,
            1,      // soft_resample
            100000, // 100ms latency
        )
    };
    if err < 0 {
        unsafe { snd_pcm_close(handle); }
        return Err(format!("snd_pcm_set_params: {}", alsa_error(err)));
    }

    Ok(PcmHandle { handle, name: name.to_string() })
}

/// List available ALSA playback devices by parsing /proc/asound/cards.
pub fn get_available_devices() -> Vec<(String, String)> {
    let mut devices = Vec::new();

    if let Ok(content) = std::fs::read_to_string("/proc/asound/cards") {
        let re = regex_lite::Regex::new(r"^\s*(\d+)\s+\[.*?\]:\s*(.*?)\s+-\s+(.*)").unwrap();
        for line in content.lines() {
            if let Some(caps) = re.captures(line) {
                let card_num = &caps[1];
                let id = caps[2].trim();
                let name = caps[3].trim();
                let alsa_name = format!("plughw:{},0", card_num);
                let display = format!("{} ({})", name, id);
                devices.push((display, alsa_name));
            }
        }
    }

    devices
}
