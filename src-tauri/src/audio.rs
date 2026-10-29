//! Captures system audio (WASAPI loopback) and the microphone, mixes them to
//! 48 kHz stereo f32, and writes the result to FFmpeg's stdin at real-time pace.
//!
//! The pump owns the audio clock. It writes silence when a device delivers
//! nothing (WASAPI loopback goes quiet when no sound plays), so FFmpeg never
//! stalls waiting for audio.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const RATE: u32 = 48_000;
/// Drop captured audio older than this, to keep latency low.
const MAX_BUFFERED_FRAMES: usize = (RATE as usize) / 5;

#[derive(Default)]
struct Source {
    /// Interleaved stereo samples at 48 kHz.
    buf: VecDeque<f32>,
    resampler: Resampler,
}

#[derive(Default)]
struct Resampler {
    pos: f64,
    prev: [f32; 2],
}

impl Resampler {
    fn push(&mut self, ratio: f64, input: &[[f32; 2]], out: &mut VecDeque<f32>) {
        if input.is_empty() {
            return;
        }
        let len = input.len() as f64;
        let frame = |i: usize| if i == 0 { self.prev } else { input[i - 1] };
        while self.pos < len {
            let i = self.pos.floor() as usize;
            let f = (self.pos - i as f64) as f32;
            let (a, b) = (frame(i), frame(i + 1));
            out.push_back(a[0] + (b[0] - a[0]) * f);
            out.push_back(a[1] + (b[1] - a[1]) * f);
            self.pos += ratio;
        }
        self.pos -= len;
        self.prev = input[input.len() - 1];
    }
}

pub struct AudioPump {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

pub struct AudioConfig {
    pub system: bool,
    pub mic: bool,
    pub mic_device: Option<String>,
    pub mic_volume: f32,
}

impl AudioPump {
    pub fn start<W: Write + Send + 'static>(out: W, cfg: AudioConfig) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("audio-pump".into())
            .spawn(move || run(out, cfg, stop2))
            .ok();
        Self { stop, handle }
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run<W: Write>(mut out: W, cfg: AudioConfig, stop: Arc<AtomicBool>) {
    let host = cpal::default_host();
    let sys = Arc::new(Mutex::new(Source::default()));
    let mic = Arc::new(Mutex::new(Source::default()));

    // Streams are not Send on Windows, so they live and die on this thread.
    let mut streams = Vec::new();
    if cfg.system {
        // On WASAPI, an input stream on an output device is a loopback capture.
        match host.default_output_device().map(|d| open_stream(&d, false, sys.clone())) {
            Some(Ok(s)) => streams.push(s),
            Some(Err(e)) => log::warn!("system audio unavailable: {e}"),
            None => log::warn!("no output device for system audio"),
        }
    }
    if cfg.mic {
        let dev = cfg
            .mic_device
            .as_ref()
            .and_then(|want| {
                host.input_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(want.as_str()))
            })
            .or_else(|| host.default_input_device());
        match dev.map(|d| open_stream(&d, true, mic.clone())) {
            Some(Ok(s)) => streams.push(s),
            Some(Err(e)) => log::warn!("microphone unavailable: {e}"),
            None => log::warn!("no microphone found"),
        }
    }

    // Block until FFmpeg starts reading, so audio time zero matches video time zero.
    let chunk = (RATE / 100) as usize; // 10 ms
    if out.write_all(&vec![0u8; chunk * 2 * 4]).is_err() {
        return;
    }
    sys.lock().buf.clear();
    mic.lock().buf.clear();

    let start = Instant::now();
    let mut written: u64 = chunk as u64;
    let mut mix = Vec::<f32>::new();
    let mut bytes = Vec::<u8>::new();
    let mic_gain = cfg.mic_volume;

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10));
        let due = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
        if due <= written {
            continue;
        }
        let frames = (due - written) as usize;
        mix.clear();
        mix.resize(frames * 2, 0.0);
        drain_into(&sys, &mut mix, 1.0, frames);
        drain_into(&mic, &mut mix, mic_gain, frames);

        bytes.clear();
        bytes.reserve(mix.len() * 4);
        for s in &mix {
            bytes.extend_from_slice(&s.clamp(-1.0, 1.0).to_le_bytes());
        }
        if out.write_all(&bytes).is_err() {
            break; // FFmpeg exited.
        }
        written = due;
    }
    for s in &streams {
        let _ = s.pause();
    }
    drop(streams);
}

fn drain_into(src: &Mutex<Source>, mix: &mut [f32], gain: f32, frames: usize) {
    let mut s = src.lock();
    // Too much backlog means the device clock runs fast or we stalled: skip ahead.
    let keep = (frames + MAX_BUFFERED_FRAMES) * 2;
    if s.buf.len() > keep {
        let extra = s.buf.len() - keep;
        s.buf.drain(..extra - extra % 2);
    }
    let n = mix.len().min(s.buf.len());
    for (m, v) in mix.iter_mut().zip(s.buf.drain(..n)) {
        *m += v * gain;
    }
}

fn open_stream(dev: &cpal::Device, is_input: bool, sink: Arc<Mutex<Source>>) -> Result<cpal::Stream, String> {
    let supported = if is_input { dev.default_input_config() } else { dev.default_output_config() }
        .map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels.max(1) as usize;
    let ratio = config.sample_rate.0 as f64 / RATE as f64;
    let err = |e| log::warn!("audio stream error: {e}");

    let stream = match format {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            &config,
            move |data: &[f32], _| feed(&sink, data, channels, ratio, |x| x),
            err,
            None,
        ),
        cpal::SampleFormat::I16 => dev.build_input_stream(
            &config,
            move |data: &[i16], _| feed(&sink, data, channels, ratio, |x| x as f32 / 32768.0),
            err,
            None,
        ),
        cpal::SampleFormat::I32 => dev.build_input_stream(
            &config,
            move |data: &[i32], _| feed(&sink, data, channels, ratio, |x| x as f32 / 2_147_483_648.0),
            err,
            None,
        ),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn feed<T: Copy>(sink: &Mutex<Source>, data: &[T], channels: usize, ratio: f64, conv: impl Fn(T) -> f32) {
    let frames: Vec<[f32; 2]> = data
        .chunks_exact(channels)
        .map(|f| {
            let l = conv(f[0]);
            let r = if channels > 1 { conv(f[1]) } else { l };
            [l, r]
        })
        .collect();
    let mut s = sink.lock();
    let Source { buf, resampler } = &mut *s;
    resampler.push(ratio, &frames, buf);
}

pub fn list_inputs() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
