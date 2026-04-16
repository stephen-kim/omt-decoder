use std::ffi::CString;
use std::sync::mpsc;

// ── ALSA FFI ──────────────────────────────────────────────────────────────

#[link(name = "asound")]
extern "C" {}

const SND_PCM_STREAM_PLAYBACK: libc::c_int = 0;
const SND_PCM_FORMAT_S16_LE: libc::c_int = 2;
const SND_PCM_ACCESS_RW_INTERLEAVED: libc::c_int = 3;

extern "C" {
    fn snd_pcm_open(pcm: *mut *mut libc::c_void, name: *const libc::c_char, stream: libc::c_int, mode: libc::c_int) -> libc::c_int;
    fn snd_pcm_set_params(pcm: *mut libc::c_void, format: libc::c_int, access: libc::c_int, channels: libc::c_uint, rate: libc::c_uint, soft_resample: libc::c_int, latency: libc::c_uint) -> libc::c_int;
    fn snd_pcm_writei(pcm: *mut libc::c_void, buffer: *const libc::c_void, size: libc::c_ulong) -> libc::c_long;
    fn snd_pcm_recover(pcm: *mut libc::c_void, err: libc::c_int, silent: libc::c_int) -> libc::c_int;
    fn snd_pcm_close(pcm: *mut libc::c_void) -> libc::c_int;
    fn snd_strerror(errnum: libc::c_int) -> *const libc::c_char;
}

fn alsa_error(err: libc::c_int) -> String {
    unsafe {
        let ptr = snd_strerror(err);
        if ptr.is_null() { format!("ALSA error {}", err) }
        else { std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string() }
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
        }
    }
}

// ── Audio Player ──────────────────────────────────────────────────────────
// Mirrors C# AudioPlayer exactly:
// - Main thread: enqueue() converts planar→interleaved, sends via channel
// - Playback thread: recv, writei to ALSA. Sleep(1ms) when empty.
// - Channel replaces C# ConcurrentQueue (lock-free MPSC)

enum AudioMsg {
    Data(Vec<i16>, u32), // (interleaved samples, frames)
    SetDevices(Vec<String>),
    SetFormat(u32, u32), // (channels, sample_rate)
    Stop,
}

pub struct AudioPlayer {
    tx: mpsc::Sender<AudioMsg>,
    channels: u32,
    rate: u32,
    volume: f32,
    device_names: Vec<String>,
}

impl AudioPlayer {
    pub fn new(device_names: &[String], volume: f32) -> Self {
        let (tx, rx) = mpsc::channel::<AudioMsg>();

        let initial_devices = device_names.to_vec();
        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                playback_thread(rx, initial_devices);
            })
            .expect("failed to spawn audio thread");

        AudioPlayer {
            tx,
            channels: 0,
            rate: 0,
            volume,
            device_names: device_names.to_vec(),
        }
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 2.0);
    }

    pub fn set_devices(&mut self, device_names: &[String]) {
        self.device_names = device_names.to_vec();
        self.channels = 0; // force re-open
        let _ = self.tx.send(AudioMsg::SetDevices(device_names.to_vec()));
    }

    pub fn enqueue(
        &mut self,
        planar_data: &[u8],
        channels: u32,
        samples_per_channel: u32,
        sample_rate: u32,
    ) {
        // Tell playback thread to re-open if format changed
        if channels != self.channels || sample_rate != self.rate {
            self.channels = channels;
            self.rate = sample_rate;
            let _ = self.tx.send(AudioMsg::SetFormat(channels, sample_rate));
        }

        let total_samples = (channels * samples_per_channel) as usize;
        let required_bytes = total_samples * std::mem::size_of::<f32>();
        if planar_data.len() < required_bytes {
            return;
        }

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

        // Non-blocking send — matches C# ConcurrentQueue.Enqueue
        let _ = self.tx.send(AudioMsg::Data(interleaved, samples_per_channel));
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(AudioMsg::Stop);
    }
}

fn playback_thread(rx: mpsc::Receiver<AudioMsg>, initial_devices: Vec<String>) {
    let mut handles: Vec<PcmHandle> = Vec::new();
    let mut device_names = initial_devices;
    let mut channels: u32 = 0;
    let mut rate: u32 = 0;

    loop {
        // Try to receive — non-blocking first, then sleep if empty (matching C#)
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            Err(mpsc::TryRecvError::Disconnected) => return,
        };

        match msg {
            AudioMsg::Stop => return,
            AudioMsg::SetDevices(names) => {
                handles.clear();
                device_names = names;
                channels = 0; // will re-open on next SetFormat or Data
            }
            AudioMsg::SetFormat(ch, sr) => {
                channels = ch;
                rate = sr;
                open_devices(&device_names, channels, rate, &mut handles);
            }
            AudioMsg::Data(buf, frames) => {
                if handles.is_empty() && channels > 0 && rate > 0 {
                    open_devices(&device_names, channels, rate, &mut handles);
                }
                let frame_count = frames as libc::c_ulong;
                for dev in &handles {
                    let err = unsafe {
                        snd_pcm_writei(dev.handle, buf.as_ptr() as *const libc::c_void, frame_count)
                    };
                    if err < 0 {
                        unsafe { snd_pcm_recover(dev.handle, err as libc::c_int, 1); }
                    }
                }
            }
        }
    }
}

fn open_devices(names: &[String], channels: u32, rate: u32, handles: &mut Vec<PcmHandle>) {
    handles.clear();
    for name in names {
        match open_pcm(name, channels, rate) {
            Ok(h) => {
                println!("Audio opened ({}): {}ch {}Hz", name, channels, rate);
                handles.push(h);
            }
            Err(e) => eprintln!("Audio open error ({}): {}", name, e),
        }
    }
}

fn open_pcm(name: &str, channels: u32, sample_rate: u32) -> Result<PcmHandle, String> {
    let cname = CString::new(name).map_err(|e| e.to_string())?;
    let mut handle: *mut libc::c_void = std::ptr::null_mut();

    let err = unsafe { snd_pcm_open(&mut handle, cname.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0) };
    if err < 0 { return Err(format!("snd_pcm_open: {}", alsa_error(err))); }

    // Match C# exactly: soft_resample=1, latency=50000us (50ms)
    let err = unsafe {
        snd_pcm_set_params(handle, SND_PCM_FORMAT_S16_LE, SND_PCM_ACCESS_RW_INTERLEAVED,
            channels, sample_rate, 1, 50000)
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
                devices.push((format!("{} ({})", name, id), format!("plughw:{},0", card_num)));
            }
        }
    }
    devices
}
