// glyph-whisper-sidecar: a resident ASR engine process wrapping whisper.cpp
// (via whisper-rs). Same wire protocol as glyph-asr-sidecar (parakeet), so the
// engine treats the two backends identically:
//
//   stdin  : binary frames  [tag:u8][len:u32 LE][payload]
//              tag 1 AUDIO   payload = raw f32 LE PCM, 16 kHz mono
//              tag 2 END     run inference on the accumulated utterance, emit final
//              tag 3 QUIT    exit
//   stdout : one JSON object per line
//              {"type":"ready"}
//              {"type":"partial","text":"..."}   (whisper is batch; we don't emit these)
//              {"type":"final","text":"..."}
//              {"type":"error","msg":"..."}
//   stderr : diagnostics + ggml backend logs (never on stdout)
//
// Backend is chosen at COMPILE time via a cargo feature (default = Vulkan for
// AMD/cross-vendor; `--features cuda` for NVIDIA). The runtime `--device`
// selects which GPU device to load on, or `cpu` to force CPU.
//
// Whisper is a batch model (it transcribes a whole buffer, not incremental
// chunks), so unlike parakeet there are no live partials: we accumulate PCM
// during the hold and transcribe once on END. For the turbo model this is well
// under a second for a short utterance.

use std::io::{Read, Write};
use std::path::PathBuf;

use serde_json::{json, Value};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

struct Args {
    model: PathBuf,
    lang: String,
    /// "cpu" forces CPU; otherwise load onto a GPU device (index, default 0).
    device: Option<String>,
    file: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut model = None;
    let mut lang = "en".to_string();
    let mut device = None;
    let mut file = None;
    let a: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < a.len() {
        let need = |i: usize| -> Result<String, String> {
            a.get(i + 1).cloned().ok_or_else(|| format!("missing value for {}", a[i]))
        };
        match a[i].as_str() {
            "--model" => model = Some(PathBuf::from(need(i)?)),
            "--lang" => lang = need(i)?,
            "--device" => device = Some(need(i)?),
            "--file" => file = Some(PathBuf::from(need(i)?)),
            "--dll" => {
                // Accepted for parity with the parakeet sidecar CLI (ignored).
                let _ = need(i)?;
            }
            other => return Err(format!("unknown arg {other}")),
        }
        i += 2;
    }
    Ok(Args { model: model.ok_or("--model required")?, lang, device, file })
}

fn emit(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// Build context params from the `--device` flag: `cpu` => CPU, anything else
/// (or unset) => GPU device 0. The actual GPU backend (Vulkan vs CUDA) is fixed
/// at compile time by the cargo feature.
fn context_params(device: &Option<String>) -> WhisperContextParameters {
    let mut p = WhisperContextParameters::default();
    let use_cpu = device.as_deref().map(|d| d.eq_ignore_ascii_case("cpu")).unwrap_or(false);
    p.use_gpu = !use_cpu;
    // GPU device index: "Vulkan0"/"CUDA0" -> 0; a bare number is honored.
    if let Some(dev) = device {
        let idx = dev
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .parse::<i32>()
            .unwrap_or(0);
        p.gpu_device = idx;
    }
    p
}

/// Transcribe 16 kHz mono f32 samples and return the joined text.
fn transcribe(ctx: &WhisperContext, samples: &[f32], lang: &str) -> Result<String, String> {
    let mut state = ctx.create_state().map_err(|e| format!("create_state: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(if lang == "auto" { "en" } else { lang }));
    // Keep it fast + avoid hallucinated trailing text.
    params.set_n_threads(num_threads());
    params.set_translate(false);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    state
        .full(params, samples)
        .map_err(|e| format!("full: {e}"))?;
    let n = state.full_n_segments();
    let mut text = String::new();
    for i in 0..n {
        let seg = match state.get_segment(i) {
            Some(s) => s,
            None => continue,
        };
        let s = seg.to_str_lossy().map_err(|e| format!("segment {i}: {e}"))?;
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(s.trim());
    }
    Ok(text)
}

fn num_threads() -> std::os::raw::c_int {
    std::thread::available_parallelism()
        .map(|n| n.get() as std::os::raw::c_int)
        .unwrap_or(4)
        .min(8)
}

fn main() {
    if let Err(e) = run() {
        emit(json!({"type":"error","msg":e}));
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    if !args.model.exists() {
        return Err(format!("model not found: {}", args.model.display()));
    }

    let mut params = context_params(&args.device);
    let backend = if params.use_gpu {
        if cfg!(feature = "cuda") { "CUDA" } else { "Vulkan" }
    } else {
        "CPU"
    };
    eprintln!(
        "[whisper] loading {} on {} (use_gpu={}, device={:?})",
        args.model.display(),
        backend,
        params.use_gpu,
        params.gpu_device
    );
    // Suppress the (very chatty) ggml progress prints on stdout — they'd corrupt
    // our line-JSON protocol.
    params.flash_attn = false;
    let ctx = WhisperContext::new_with_params(&args.model, params)
        .map_err(|e| format!("load model: {e}"))?;
    eprintln!("[whisper] model loaded");

    // One-shot file mode: transcribe a WAV, print final, exit.
    if let Some(wav) = &args.file {
        let samples = read_wav_mono_f32(wav)?;
        let text = transcribe(&ctx, &samples, &args.lang)?;
        emit(json!({"type":"final","text":text}));
        return Ok(());
    }

    // Streaming mode.
    emit(json!({"type":"ready"}));

    let mut audio: Vec<f32> = Vec::new();
    let mut stdin = std::io::stdin().lock();

    loop {
        let mut hdr = [0u8; 5];
        match stdin.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(_) => break, // EOF / pipe closed
        }
        let tag = hdr[0];
        let len = u32::from_le_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
        let mut payload = vec![0u8; len];
        if len > 0 && stdin.read_exact(&mut payload).is_err() {
            break;
        }

        match tag {
            1 => {
                // AUDIO: f32 LE samples, append to the running utterance buffer.
                audio.extend(payload.chunks_exact(4).map(|b| {
                    f32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }));
            }
            2 => {
                // END: transcribe the accumulated utterance, then reset the buffer.
                let text = if audio.is_empty() {
                    String::new()
                } else {
                    match transcribe(&ctx, &audio, &args.lang) {
                        Ok(t) => t,
                        Err(e) => {
                            audio.clear();
                            emit(json!({"type":"error","msg":e}));
                            emit(json!({"type":"final","text":""}));
                            continue;
                        }
                    }
                };
                audio.clear();
                emit(json!({"type":"final","text":text}));
            }
            3 => break, // QUIT
            other => emit(json!({"type":"error","msg":format!("bad tag {other}")})),
        }
    }

    Ok(())
}

/// Read a WAV file and return mono f32 samples resampled-free (caller expects
/// 16 kHz mono; whisper.cpp also expects 16 kHz mono, which the GGUFs are built
/// for). Downmixes to mono by averaging channels.
fn read_wav_mono_f32(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("open wav: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if channels == 0 {
        return Err("wav has 0 channels".into());
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.into_samples::<f32>().map(|s| s.unwrap_or(0.0)).collect()
        }
        hound::SampleFormat::Int => {
            let max = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader.into_samples::<i32>().map(|s| s.unwrap_or(0) as f32 / max).collect()
        }
        _ => return Err("unsupported wav sample format".into()),
    };
    if channels == 1 {
        Ok(samples)
    } else {
        Ok(samples
            .chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect())
    }
}
