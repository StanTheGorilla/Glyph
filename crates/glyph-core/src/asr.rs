//! ASR engine trait + parakeet sidecar client.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{anyhow, bail, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Partial,
    Final,
}

/// One transcript update. In streaming, `text` is a DELTA — the text newly
/// finalized since the previous event, not the cumulative transcript — so the
/// consumer accumulates deltas into the running transcript. `Final` carries the
/// end-of-utterance tail. `text` has the model's trailing `<lang>` tag removed;
/// the tag (e.g. "en-US") is surfaced separately in `lang_tag`.
#[derive(Debug, Clone)]
pub struct TranscriptEvent {
    pub kind: EventKind,
    pub text: String,
    pub lang_tag: Option<String>,
}

/// Backend-agnostic streaming ASR engine.
pub trait AsrEngine {
    /// Offline transcription of a finished WAV. Returns the full transcript.
    fn transcribe_file(&self, wav: &Path, lang: &str) -> Result<String>;
    /// Begin a live streaming session; feed 16 kHz mono f32 PCM into the handle.
    fn start_stream(&self, cfg: StreamConfig) -> Result<StreamHandle>;
}

#[derive(Clone)]
pub struct StreamConfig {
    /// Locale prompt for the multilingual nemotron model ("en", "de", "auto", ...).
    pub lang: String,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self { lang: "auto".into() }
    }
}

/// An `AsrEngine` backed by the `glyph-asr-sidecar` subprocess (parakeet.dll).
#[derive(Clone)]
pub struct SidecarEngine {
    /// Path to the glyph-asr-sidecar executable.
    pub sidecar: PathBuf,
    /// Path to parakeet.dll (the Vulkan build doubles as CPU via `device`).
    pub dll: PathBuf,
    /// Path to the ASR GGUF model.
    pub model: PathBuf,
    /// ggml device override: None = auto (first GPU), or "Vulkan0" / "cpu".
    pub device: Option<String>,
}

impl SidecarEngine {
    pub fn new(sidecar: PathBuf, dll: PathBuf, model: PathBuf) -> Self {
        Self { sidecar, dll, model, device: None }
    }

    fn base_command(&self, lang: &str) -> Command {
        let mut cmd = Command::new(&self.sidecar);
        cmd.arg("--model").arg(&self.model);
        cmd.arg("--dll").arg(&self.dll);
        cmd.arg("--lang").arg(lang);
        if let Some(dev) = &self.device {
            cmd.arg("--device").arg(dev);
        }
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: a GUI app (no console of its own) would otherwise
            // pop a console window for this console-subsystem sidecar.
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd
    }
}

impl AsrEngine for SidecarEngine {
    fn transcribe_file(&self, wav: &Path, lang: &str) -> Result<String> {
        let out = self
            .base_command(lang)
            .arg("--file")
            .arg(wav)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .context("spawn sidecar (file mode)")?;
        if !out.status.success() {
            bail!("sidecar exited with {}", out.status);
        }
        // The sidecar prints exactly one JSON line in file mode.
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let v: serde_json::Value = serde_json::from_str(line).context("parse sidecar json")?;
            match v.get("type").and_then(|t| t.as_str()) {
                Some("final") => {
                    return Ok(v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string());
                }
                Some("error") => bail!("sidecar error: {}", v.get("msg").and_then(|m| m.as_str()).unwrap_or("?")),
                _ => {}
            }
        }
        bail!("sidecar produced no final result")
    }

    fn start_stream(&self, cfg: StreamConfig) -> Result<StreamHandle> {
        let mut child = self
            .base_command(&cfg.lang)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawn sidecar (stream mode)")?;
        // Tie the sidecar to the kill-job so it dies with us (Drop alone misses a
        // Task Manager kill / Quit-via-process::exit).
        crate::proc_guard::guard(&child);

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no child stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no child stdout"))?;

        let (tx, rx) = mpsc::channel::<TranscriptEvent>();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let reader = std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            for line in lines.by_ref() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                let kind = match v.get("type").and_then(|t| t.as_str()) {
                    Some("partial") => EventKind::Partial,
                    Some("final") => EventKind::Final,
                    Some("ready") => {
                        let _ = ready_tx.send(());
                        continue;
                    }
                    Some("error") => {
                        eprintln!("[asr] sidecar error: {}", v.get("msg").and_then(|m| m.as_str()).unwrap_or("?"));
                        continue;
                    }
                    _ => continue,
                };
                let raw = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let (text, lang_tag) = split_lang_tag(raw);
                if tx.send(TranscriptEvent { kind, text, lang_tag }).is_err() {
                    break;
                }
            }
        });

        // Block until the engine reports the model is loaded, so the first
        // utterance isn't delayed by model load + Vulkan shader compilation.
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .context("engine did not become ready (model load failed?)")?;

        Ok(StreamHandle {
            writer: StreamWriter { stdin: Arc::new(Mutex::new(stdin)) },
            child,
            events: rx,
            reader: Some(reader),
        })
    }
}

/// The write side of a stream: feed PCM / mark utterance end / quit. Cloneable and
/// `Send + Sync`, so an audio feeder thread can hold one while the control thread
/// keeps the `StreamHandle` (and its non-`Sync` events `Receiver`).
#[derive(Clone)]
pub struct StreamWriter {
    stdin: Arc<Mutex<ChildStdin>>,
}

impl StreamWriter {
    /// Feed a block of 16 kHz mono f32 PCM.
    pub fn feed(&self, pcm: &[f32]) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(pcm.len() * 4);
        for s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.write_frame(1, &bytes)
    }

    /// Finalize the current utterance (engine flushes its tail and emits Final).
    pub fn end_utterance(&self) -> Result<()> {
        self.write_frame(2, &[])
    }

    /// Tell the engine process to quit.
    pub fn quit(&self) -> Result<()> {
        self.write_frame(3, &[])
    }

    fn write_frame(&self, tag: u8, payload: &[u8]) -> Result<()> {
        let mut w = self.stdin.lock().unwrap();
        w.write_all(&[tag])?;
        w.write_all(&(payload.len() as u32).to_le_bytes())?;
        w.write_all(payload)?;
        w.flush()?;
        Ok(())
    }
}

/// Live streaming session. Feed PCM, mark utterance end, read events.
pub struct StreamHandle {
    writer: StreamWriter,
    child: Child,
    /// Transcript events (Partial as audio is decoded, Final on `end_utterance`).
    pub events: Receiver<TranscriptEvent>,
    reader: Option<JoinHandle<()>>,
}

impl StreamHandle {
    /// A cloneable write handle for a feeder thread.
    pub fn writer(&self) -> StreamWriter {
        self.writer.clone()
    }

    /// Move the events receiver out (e.g. into a collector thread), leaving the
    /// handle otherwise intact for `writer()` / `finish()`.
    pub fn take_events(&mut self) -> Receiver<TranscriptEvent> {
        let (_dead, rx) = mpsc::channel();
        std::mem::replace(&mut self.events, rx)
    }

    /// Feed a block of 16 kHz mono f32 PCM.
    pub fn feed(&self, pcm: &[f32]) -> Result<()> {
        self.writer.feed(pcm)
    }

    /// Finalize the current utterance.
    pub fn end_utterance(&self) -> Result<()> {
        self.writer.end_utterance()
    }

    /// Tell the engine to quit and wait for it to exit.
    pub fn finish(mut self) -> Result<()> {
        let _ = self.writer.quit();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        let _ = self.child.wait();
        Ok(())
    }
}

/// Split a trailing `<locale>` tag (e.g. "hello <en-US>") into (text, Some("en-US")).
fn split_lang_tag(raw: &str) -> (String, Option<String>) {
    let t = raw.trim_end();
    if t.ends_with('>') {
        if let Some(open) = t.rfind('<') {
            let tag = &t[open + 1..t.len() - 1];
            if !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return (t[..open].trim_end().to_string(), Some(tag.to_string()));
            }
        }
    }
    (raw.to_string(), None)
}
