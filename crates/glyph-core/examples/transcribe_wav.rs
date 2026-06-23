//! Phase 1 milestone: Rust gets correct text for a test WAV via the parakeet
//! sidecar on Vulkan — exercised through BOTH the offline file path and the live
//! streaming path (PCM frames -> JSON events).
//!
//! Paths default to the Phase 0 scratch artifacts; override with env vars:
//!   GLYPH_DLL, GLYPH_MODEL, GLYPH_WAV, GLYPH_SIDECAR

use std::path::PathBuf;
use std::time::Duration;

use glyph_core::{AsrEngine, EventKind, SidecarEngine, StreamConfig};

fn env_path(key: &str, default: &str) -> PathBuf {
    std::env::var(key).map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(default))
}

/// Locate the sidecar next to this example binary (target/debug/glyph-asr-sidecar.exe).
fn default_sidecar() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe.parent().and_then(|p| p.parent()).expect("target/debug");
    dir.join(if cfg!(windows) { "glyph-asr-sidecar.exe" } else { "glyph-asr-sidecar" })
}

fn read_wav_16k_mono(path: &PathBuf) -> Vec<f32> {
    let mut r = hound::WavReader::open(path).expect("open wav");
    let spec = r.spec();
    assert_eq!(spec.channels, 1, "expected mono test wav");
    assert_eq!(spec.sample_rate, 16000, "expected 16 kHz test wav");
    match spec.sample_format {
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
    }
}

fn main() -> anyhow::Result<()> {
    let sidecar = env_path("GLYPH_SIDECAR", default_sidecar().to_str().unwrap());
    let dll = env_path(
        "GLYPH_DLL",
        r"F:\Glyph\phase0\lib-vulkan\parakeet-v0.3.2-lib-win-vulkan-x64\parakeet.dll",
    );
    let model = env_path(
        "GLYPH_MODEL",
        r"F:\Glyph\phase0\models\nemotron-3.5-asr-streaming-0.6b-q8_0.gguf",
    );
    let wav = env_path("GLYPH_WAV", r"F:\Glyph\phase0\audio\test_tts.wav");

    let engine = SidecarEngine::new(sidecar, dll, model);

    // --- (1) offline file path ---
    println!("== transcribe_file ==");
    let text = engine.transcribe_file(&wav, "en")?;
    println!("text: {text}");
    assert!(
        text.to_lowercase().contains("quick brown fox"),
        "unexpected transcript: {text}"
    );
    println!("file path OK\n");

    // --- (2) live streaming path ---
    println!("== start_stream (feed PCM in 0.2s chunks) ==");
    let pcm = read_wav_16k_mono(&wav);
    let handle = engine.start_stream(StreamConfig { lang: "en".into() })?;

    let chunk = 16000 / 5; // 0.2 s
    for block in pcm.chunks(chunk) {
        handle.feed(block)?;
    }
    handle.end_utterance()?;

    // Events carry DELTAS (newly-finalized text). The running transcript is their
    // concatenation; the Final event is just the end-of-utterance tail.
    let mut transcript = String::new();
    loop {
        match handle.events.recv_timeout(Duration::from_secs(5)) {
            Ok(ev) => {
                match ev.kind {
                    EventKind::Partial => println!("partial: {:?}", ev.text),
                    EventKind::Final => {
                        println!("FINAL tail: {:?}  [lang={:?}]", ev.text, ev.lang_tag);
                        transcript.push_str(&ev.text);
                        break;
                    }
                }
                transcript.push_str(&ev.text);
            }
            Err(_) => {
                eprintln!("timed out waiting for final");
                break;
            }
        }
    }
    handle.finish()?;

    let transcript = transcript.trim();
    println!("\naccumulated transcript: {transcript}");
    assert!(
        transcript.to_lowercase().contains("quick brown fox"),
        "unexpected streamed transcript: {transcript}"
    );
    println!("streaming path OK");
    Ok(())
}
