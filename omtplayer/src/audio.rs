use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::Direction;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct PcmDevice {
    pcm: PCM,
    name: String,
}

pub struct AudioPlayer {
    devices: Arc<Mutex<Vec<String>>>,
    pcm_handles: Arc<Mutex<Vec<PcmDevice>>>,
    queue: Arc<Mutex<VecDeque<Vec<f32>>>>,
    channels: Arc<Mutex<u32>>,
    rate: Arc<Mutex<u32>>,
    running: Arc<Mutex<bool>>,
}

impl AudioPlayer {
    pub fn new(device_names: &[String]) -> Self {
        let player = AudioPlayer {
            devices: Arc::new(Mutex::new(device_names.to_vec())),
            pcm_handles: Arc::new(Mutex::new(Vec::new())),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            channels: Arc::new(Mutex::new(0)),
            rate: Arc::new(Mutex::new(0)),
            running: Arc::new(Mutex::new(true)),
        };

        // Start playback thread
        let queue = player.queue.clone();
        let handles = player.pcm_handles.clone();
        let channels = player.channels.clone();
        let running = player.running.clone();

        std::thread::spawn(move || {
            playback_loop(queue, handles, channels, running);
        });

        player
    }

    pub fn set_devices(&mut self, device_names: &[String]) {
        let mut devices = self.devices.lock().unwrap();
        *devices = device_names.to_vec();
        // Force re-open
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
        // Check if we need to re-open devices
        {
            let current_ch = *self.channels.lock().unwrap();
            let current_rate = *self.rate.lock().unwrap();
            let handles = self.pcm_handles.lock().unwrap();
            if current_ch != channels || current_rate != sample_rate || handles.is_empty() {
                drop(handles);
                self.open_audio(channels, sample_rate);
            }
        }

        // Planar float → interleaved float
        let total_samples = (channels * samples_per_channel) as usize;
        let mut interleaved = vec![0.0f32; total_samples];

        let src =
            unsafe { std::slice::from_raw_parts(planar_data.as_ptr() as *const f32, total_samples) };

        for s in 0..samples_per_channel as usize {
            for c in 0..channels as usize {
                interleaved[s * channels as usize + c] = src[c * samples_per_channel as usize + s];
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
            match open_pcm_device(name, channels, sample_rate) {
                Ok(pcm) => {
                    println!("Audio opened ({}): {}ch {}Hz", name, channels, sample_rate);
                    handles.push(PcmDevice {
                        pcm,
                        name: name.clone(),
                    });
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

fn open_pcm_device(name: &str, channels: u32, sample_rate: u32) -> Result<PCM, alsa::Error> {
    let pcm = PCM::new(name, Direction::Playback, false)?;
    {
        let hwp = HwParams::any(&pcm)?;
        hwp.set_access(Access::RWInterleaved)?;
        hwp.set_format(Format::FloatLE)?;
        hwp.set_channels(channels)?;
        hwp.set_rate(sample_rate, alsa::ValueOr::Nearest)?;
        hwp.set_buffer_time_near(50000, alsa::ValueOr::Nearest)?;
        pcm.hw_params(&hwp)?;
    }
    pcm.prepare()?;
    Ok(pcm)
}

fn playback_loop(
    queue: Arc<Mutex<VecDeque<Vec<f32>>>>,
    handles: Arc<Mutex<Vec<PcmDevice>>>,
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
            let handles = handles.lock().unwrap();
            for dev in handles.iter() {
                let io = dev.pcm.io_f32().ok();
                if let Some(io) = io {
                    match io.writei(&buf) {
                        Ok(_) => {}
                        Err(e) => {
                            let _ = dev.pcm.recover(e.errno() as i32, true);
                        }
                    }
                }
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

/// List available ALSA playback devices by parsing /proc/asound/cards.
/// Returns a list of (display_name, alsa_device) pairs.
pub fn get_available_devices() -> Vec<(String, String)> {
    let mut devices = vec![("Default".to_string(), "default".to_string())];

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
