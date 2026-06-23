//! Mic capture (cpal/WASAPI) + resample to 16 kHz mono (rubato).
//!
//! Design: the realtime audio callback only downmixes to mono and forwards RAW
//! device-rate blocks over a channel (cheap, no allocations of FFT plans). A
//! consumer drives a persistent `Resampler16k` to produce 16 kHz mono for ASR.

use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use rubato::{FftFixedIn, Resampler};

pub const TARGET_RATE: usize = 16_000;
const RS_CHUNK: usize = 1024; // device-rate input frames per resample step

/// Offline one-shot resample of mono `input` at `in_rate` to 16 kHz mono.
pub fn resample_to_16k(input: &[f32], in_rate: u32) -> Result<Vec<f32>> {
    let mut rs = Resampler16k::new(in_rate)?;
    let mut out = rs.push(input)?;
    out.extend(rs.flush()?);
    Ok(out)
}

/// Persistent streaming resampler: device-rate mono in, 16 kHz mono out.
pub struct Resampler16k {
    rs: Option<FftFixedIn<f32>>, // None when device is already 16 kHz
    buf: Vec<f32>,
}

impl Resampler16k {
    pub fn new(in_rate: u32) -> Result<Self> {
        let rs = if in_rate as usize == TARGET_RATE {
            None
        } else {
            Some(
                FftFixedIn::<f32>::new(in_rate as usize, TARGET_RATE, RS_CHUNK, 1, 1)
                    .context("create resampler")?,
            )
        };
        Ok(Self { rs, buf: Vec::new() })
    }

    /// Feed raw device-rate mono; returns whatever 16 kHz mono is produced now.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        let Some(rs) = self.rs.as_mut() else {
            return Ok(samples.to_vec());
        };
        self.buf.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.buf.len() >= RS_CHUNK {
            let frame: Vec<f32> = self.buf.drain(..RS_CHUNK).collect();
            let o = rs.process(&[frame], None).context("resample")?;
            out.extend_from_slice(&o[0]);
        }
        Ok(out)
    }

    /// Flush any buffered tail (zero-padded) at end of utterance.
    pub fn flush(&mut self) -> Result<Vec<f32>> {
        let Some(rs) = self.rs.as_mut() else {
            return Ok(Vec::new());
        };
        if self.buf.is_empty() {
            return Ok(Vec::new());
        }
        let mut frame = vec![0f32; RS_CHUNK];
        let n = self.buf.len().min(RS_CHUNK);
        frame[..n].copy_from_slice(&self.buf[..n]);
        self.buf.clear();
        let o = rs.process(&[frame], None).context("resample flush")?;
        Ok(o[0].clone())
    }
}

fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// A live capture session. Keep it alive to keep the mic open; the RAW
/// device-rate mono blocks are delivered on the separately-returned `Receiver`
/// (cpal's `Stream` is `!Send` and `Drop`, so the channel is handed out apart
/// from the stream keeper).
pub struct CaptureStream {
    _stream: cpal::Stream,
    pub device_rate: u32,
    pub channels: u16,
    pub name: String,
}

/// Pick an input device: first whose name contains `filter` (case-insensitive),
/// else the system default.
fn pick_input(host: &cpal::Host, filter: Option<&str>) -> Result<cpal::Device> {
    if let Some(f) = filter.filter(|s| !s.trim().is_empty()) {
        let fl = f.to_lowercase();
        for d in host.input_devices()? {
            if let Ok(n) = d.name() {
                if n.to_lowercase().contains(&fl) {
                    return Ok(d);
                }
            }
        }
        eprintln!("[audio] no input device matching '{f}'; using default");
    }
    host.default_input_device().ok_or_else(|| anyhow!("no default input device"))
}

/// Names of available input devices (for the settings UI dropdown).
pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devs) => devs.filter_map(|d| d.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// List input devices (for the `mics` subcommand / config help).
pub fn list_mics() -> Result<()> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    println!("input devices (set audio.device in glyph.toml to a substring):");
    for d in host.input_devices()? {
        let n = d.name().unwrap_or_else(|_| "<?>".into());
        let mark = if Some(&n) == default.as_ref() { "  (default)" } else { "" };
        println!("  - {n}{mark}");
    }
    Ok(())
}

impl CaptureStream {
    pub fn start(device_filter: Option<&str>) -> Result<(CaptureStream, Receiver<Vec<f32>>)> {
        let host = cpal::default_host();
        let device = pick_input(&host, device_filter)?;
        let name = device.name().unwrap_or_else(|_| "<?>".into());
        let cfg = device.default_input_config().context("default input config")?;
        let device_rate = cfg.sample_rate().0;
        let channels = cfg.channels();
        let sample_format = cfg.sample_format();
        let stream_cfg: cpal::StreamConfig = cfg.into();
        let ch = channels as usize;

        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let err_fn = |e| eprintln!("[audio] stream error: {e}");
        let stream = match sample_format {
            SampleFormat::F32 => {
                let tx: Sender<Vec<f32>> = tx;
                device.build_input_stream(
                    &stream_cfg,
                    move |data: &[f32], _| {
                        let _ = tx.send(downmix(data, ch));
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::I16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    let _ = tx.send(downmix(&f, ch));
                },
                err_fn,
                None,
            )?,
            SampleFormat::U16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0).collect();
                    let _ = tx.send(downmix(&f, ch));
                },
                err_fn,
                None,
            )?,
            other => return Err(anyhow!("unsupported sample format {other:?}")),
        };
        stream.play().context("start stream")?;
        Ok((CaptureStream { _stream: stream, device_rate, channels, name }, rx))
    }
}

/// Headless self-test: list devices, capture ~1.5 s, report stats + resampled length.
pub fn probe() -> Result<()> {
    let host = cpal::default_host();
    eprintln!("host: {:?}", host.id());
    eprintln!("input devices:");
    for d in host.input_devices()? {
        eprintln!("  - {}", d.name().unwrap_or_else(|_| "<?>".into()));
    }
    let (cap, blocks) = CaptureStream::start(None)?;
    eprintln!("default input: {} rate={} Hz, channels={}", cap.name, cap.device_rate, cap.channels);
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let mut raw = Vec::new();
    while let Ok(b) = blocks.try_recv() {
        raw.extend_from_slice(&b);
    }
    let resampled = resample_to_16k(&raw, cap.device_rate)?;
    let n = resampled.len();
    let secs = n as f32 / TARGET_RATE as f32;
    let peak = resampled.iter().fold(0f32, |m, s| m.max(s.abs()));
    let rms = (resampled.iter().map(|s| s * s).sum::<f32>() / n.max(1) as f32).sqrt();
    eprintln!(
        "raw device samples={} @{}Hz -> {} @16k ({secs:.2}s) peak={peak:.4} rms={rms:.4}",
        raw.len(),
        cap.device_rate,
        n
    );
    if n == 0 {
        return Err(anyhow!("no audio captured"));
    }
    println!("AUDIO PROBE OK: {n} samples @16k, {secs:.2}s, peak={peak:.4}, rms={rms:.4}");
    Ok(())
}

/// Raw diagnostic: count callbacks + raw samples with no resampling.
pub fn probe_raw() -> Result<()> {
    use std::sync::{Arc, Mutex};
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("no input device"))?;
    let cfg = device.default_input_config()?;
    eprintln!("raw probe: {} Hz, {} ch, {:?}", cfg.sample_rate().0, cfg.channels(), cfg.sample_format());
    let scfg: cpal::StreamConfig = cfg.into();
    let calls = Arc::new(Mutex::new((0u64, 0u64, 0f32)));
    let c2 = calls.clone();
    let stream = device.build_input_stream(
        &scfg,
        move |d: &[f32], _| {
            let mut g = c2.lock().unwrap();
            g.0 += 1;
            g.1 += d.len() as u64;
            for s in d {
                g.2 = g.2.max(s.abs());
            }
        },
        |e| eprintln!("[audio] err: {e}"),
        None,
    )?;
    stream.play()?;
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let g = *calls.lock().unwrap();
    println!("RAW PROBE: callbacks={} samples={} peak={:.4}", g.0, g.1, g.2);
    Ok(())
}
