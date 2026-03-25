use std::collections::VecDeque;
use std::ffi::CString;
use std::sync::{Arc, Mutex};

// ── ALSA FFI (matching C# P/Invoke exactly) ───────────────────────────────

const SND_PCM_STREAM_PLAYBACK: libc::c_int = 0;
const SND_PCM_FORMAT_FLOAT_LE: libc::c_int = 14;
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

// ── Audio Player ──────────────────────────────────────────────────────────

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

pub struct AudioPlayer {
    devices: Arc<Mutex<Vec<String>>>,
    pcm_handles: Arc<Mutex<Vec<PcmHandle>>>,
    queue: Arc<Mutex<VecDeque<Vec<f32>>>>,
    channels: Arc<Mutex<u32>>,
    rate: Arc<Mutex<u32>>,
    running: Arc<Mutex<bool>>,
    volume: Arc<Mutex<f32>>,
}

impl AudioPlayer {
    pub fn new(device_names: &[String], volume: f32) -> Self {
        let player = AudioPlayer {
            devices: Arc::new(Mutex::new(device_names.to_vec())),
            pcm_handles: Arc::new(Mutex::new(Vec::new())),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            channels: Arc::new(Mutex::new(0)),
            rate: Arc::new(Mutex::new(0)),
            running: Arc::new(Mutex::new(true)),
            volume: Arc::new(Mutex::new(volume)),
        };

        let queue = player.queue.clone();
        let handles = player.pcm_handles.clone();
        let channels = player.channels.clone();
        let running = player.running.clone();

        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                playback_loop(queue, handles, channels, running);
            })
            .expect("failed to spawn audio thread");

        player
    }

    pub fn set_volume(&self, vol: f32) {
        *self.volume.lock().unwrap() = vol.clamp(0.0, 2.0);
    }

    pub fn set_devices(&mut self, device_names: &[String]) {
        let mut devices = self.devices.lock().unwrap();
        *devices = device_names.to_vec();
        let mut handles = self.pcm_handles.lock().unwrap();
        handles.clear();
        *self.channels.lock().unwrap() = 0;
    }

    pub fn enqueue(
        &mut self,
        planar_data: &[u8],
        channels: u32,
        samples_per_channel: u32,
        sample_rate: u32,
    ) {
        {
            let current_ch = *self.channels.lock().unwrap();
            let current_rate = *self.rate.lock().unwrap();
            let handles = self.pcm_handles.lock().unwrap();
            if current_ch != channels || current_rate != sample_rate || handles.is_empty() {
                drop(handles);
                self.open_audio(channels, sample_rate);
            }
        }

        let total_samples = (channels * samples_per_channel) as usize;
        let required_bytes = total_samples * std::mem::size_of::<f32>();
        if planar_data.len() < required_bytes {
            return; // incomplete audio data, skip
        }

        let mut interleaved = vec![0.0f32; total_samples];
        let vol = *self.volume.lock().unwrap();

        let src = unsafe {
            std::slice::from_raw_parts(planar_data.as_ptr() as *const f32, total_samples)
        };

        for s in 0..samples_per_channel as usize {
            for c in 0..channels as usize {
                interleaved[s * channels as usize + c] =
                    src[c * samples_per_channel as usize + s] * vol;
            }
        }

        let mut queue = self.queue.lock().unwrap();
        queue.push_back(interleaved);
        while queue.len() > 10 {
            queue.pop_front();
        }
    }

    fn open_audio(&self, channels: u32, sample_rate: u32) {
        let mut handles = self.pcm_handles.lock().unwrap();
        handles.clear();

        let devices = self.devices.lock().unwrap();
        for name in devices.iter() {
            match open_pcm(name, channels, sample_rate) {
                Ok(handle) => {
                    println!("Audio opened ({}): {}ch {}Hz", name, channels, sample_rate);
                    handles.push(handle);
                }
                Err(e) => {
                    eprintln!("Audio open error ({}): {}", name, e);
                }
            }
        }

        *self.channels.lock().unwrap() = channels;
        *self.rate.lock().unwrap() = sample_rate;
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        *self.running.lock().unwrap() = false;
    }
}

/// Open ALSA device using snd_pcm_set_params — identical to C# P/Invoke path.
fn open_pcm(name: &str, channels: u32, sample_rate: u32) -> Result<PcmHandle, String> {
    let cname = CString::new(name).map_err(|e| e.to_string())?;
    let mut handle: *mut libc::c_void = std::ptr::null_mut();

    let err = unsafe {
        snd_pcm_open(&mut handle, cname.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0)
    };
    if err < 0 {
        return Err(format!("snd_pcm_open: {}", alsa_error(err)));
    }

    // Matches C#: snd_pcm_set_params(handle, FLOAT_LE, RW_INTERLEAVED, ch, rate, 1, 50000)
    let err = unsafe {
        snd_pcm_set_params(
            handle,
            SND_PCM_FORMAT_FLOAT_LE,
            SND_PCM_ACCESS_RW_INTERLEAVED,
            channels,
            sample_rate,
            1,     // soft_resample = true
            50000, // latency = 50ms
        )
    };
    if err < 0 {
        unsafe { snd_pcm_close(handle); }
        return Err(format!("snd_pcm_set_params: {}", alsa_error(err)));
    }

    Ok(PcmHandle { handle, name: name.to_string() })
}

fn playback_loop(
    queue: Arc<Mutex<VecDeque<Vec<f32>>>>,
    handles: Arc<Mutex<Vec<PcmHandle>>>,
    channels: Arc<Mutex<u32>>,
    running: Arc<Mutex<bool>>,
) {
    loop {
        if !*running.lock().unwrap() {
            break;
        }

        let buffer = {
            let mut q = queue.lock().unwrap();
            q.pop_front()
        };

        if let Some(buf) = buffer {
            let ch = *channels.lock().unwrap();
            if ch == 0 {
                continue;
            }
            let frames = buf.len() as libc::c_ulong / ch as libc::c_ulong;
            let handles = handles.lock().unwrap();
            for dev in handles.iter() {
                let err = unsafe {
                    snd_pcm_writei(dev.handle, buf.as_ptr() as *const libc::c_void, frames)
                };
                if err < 0 {
                    unsafe {
                        snd_pcm_recover(dev.handle, err as libc::c_int, 1);
                    }
                }
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
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
