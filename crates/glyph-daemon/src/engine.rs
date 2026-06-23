//! The dictation engine: orchestrates hotkey -> capture -> ASR -> cleanup ->
//! inject, emitting `EngineEvent`s to a sink. Drives both the headless bin
//! (console sink) and the Tauri app (events to the HUD).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use glyph_core::{
    AsrEngine, Cleaner, EventKind, LlamaCleaner, SidecarEngine, StreamConfig, StreamWriter,
};
use serde::Serialize;

use crate::audio::{CaptureStream, Resampler16k};
use crate::config::Config;
use crate::hotkey::{self, HotkeyEvent};
use crate::{inject, llama};

/// Events the engine emits to whatever UI/sink is attached.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineEvent {
    /// Engine finished loading (ASR model + cleanup server).
    Ready { cleanup: bool },
    /// Recording started; `app` is the focused process name.
    RecordingStarted { app: String },
    /// Live running transcript (accumulated) while recording.
    Partial { text: String },
    /// Recording stopped; cleanup/injection in progress.
    Stopped,
    /// Final result, already injected. Empty strings = nothing captured.
    Finalized { raw: String, clean: String, app: String },
    Error { message: String },
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, ev: EngineEvent);
}

impl<F> EventSink for F
where
    F: Fn(EngineEvent) + Send + Sync + 'static,
{
    fn emit(&self, ev: EngineEvent) {
        (self)(ev)
    }
}

/// Settings the engine re-reads on every finished utterance, so the UI can change
/// them with no restart: spoken-cue snippets and the cleanup dictionary terms.
#[derive(Default, Clone)]
pub struct LiveConfig {
    /// Spoken cue -> expansion, applied (case-insensitive) after cleanup.
    pub snippets: BTreeMap<String, String>,
    /// Proper nouns / terms biased into the cleanup prompt to keep spelling.
    pub dict_terms: Vec<String>,
}

/// Running engine. Dropping it tears the whole pipeline down in-process (so the
/// app can restart the engine on a model/engine change with no relaunch): the
/// cleanup server is killed, the ASR sidecar is told to quit, the global keyboard
/// hook is uninstalled, and all worker threads are joined.
pub struct Engine {
    _llama: Option<llama::LlamaServer>,
    shutdown: Option<Shutdown>,
}

/// Everything needed to stop a running engine cleanly.
struct Shutdown {
    /// ASR sidecar stdin — `quit()` makes it exit, ending the collector. `None`
    /// when the ASR model failed to load (engine ran hotkey-only).
    writer: Option<StreamWriter>,
    /// Tells the control thread to stop capturing and exit.
    stop: Arc<AtomicBool>,
    /// Win32 id of the hotkey thread, for posting WM_QUIT to its message loop.
    hk_tid: Arc<AtomicU32>,
    /// Collector, hotkey, and control threads (joined on shutdown).
    threads: Vec<thread::JoinHandle<()>>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let Some(sd) = self.shutdown.take() else { return };
        // Stop capturing and exit the control loop.
        sd.stop.store(true, Ordering::Relaxed);
        // Tell the ASR sidecar to quit; its stdout closes and the collector ends.
        if let Some(w) = &sd.writer {
            let _ = w.quit();
        }
        // Break the hotkey thread's message loop, which uninstalls the hook.
        crate::hotkey::stop_thread(sd.hk_tid.load(Ordering::Relaxed));
        for h in sd.threads {
            let _ = h.join();
        }
        // `_llama` drops here, killing the cleanup server and freeing its port.
    }
}

impl Engine {
    /// Start the engine. `hold` is the live hotkey-behavior flag (true = hold /
    /// push-to-talk, false = toggle / click-click), owned by the caller. The
    /// control thread reads it on every keypress, so the settings UI can flip it
    /// instantly through its own clone — no restart, no model reload.
    pub fn start(
        cfg: Config,
        sink: Arc<dyn EventSink>,
        hold: Arc<AtomicBool>,
        live: Arc<Mutex<LiveConfig>>,
    ) -> Result<Engine> {
        hotkey::parse_spec(&cfg.hotkey.combo)
            .map_err(|e| anyhow!("bad hotkey '{}': {e}", cfg.hotkey.combo))?;
        let hold_ctl = hold;
        let live_ctl = live;
        let method = cfg.inject_method();
        let keep_clip = cfg.inject.keep_on_clipboard;
        let mic = cfg.audio.device.clone();
        // Shutdown signal for the control thread (see `Engine`'s Drop).
        let stop = Arc::new(AtomicBool::new(false));

        // ASR sidecar (resident; model loaded once). `kind` selects parakeet vs whisper.
        let (sidecar, model) = cfg.active_asr();
        let asr = SidecarEngine {
            sidecar,
            dll: cfg.asr.dll.clone(),
            model,
            device: cfg.device_arg(),
        };
        // ASR load is non-fatal: a bad/missing model (e.g. Nemotron selected with
        // no nemotron gguf) must NOT kill the hotkey, or the user gets locked out
        // of the whole app with no way to fix it. On failure we surface the error
        // and keep the control + hotkey threads alive; recording then reports "no
        // model" until a working one is picked and the engine reloads.
        let (writer, writer_sd, events) =
            match asr.start_stream(StreamConfig { lang: cfg.asr.lang.clone() }) {
                Ok(mut handle) => {
                    let w = handle.writer();
                    let wsd = handle.writer(); // kept by Engine to quit the sidecar on drop
                    let ev = handle.take_events();
                    (Some(w), Some(wsd), Some(ev))
                }
                Err(e) => {
                    sink.emit(EngineEvent::Error {
                        message: format!("transcription model failed to load: {e}"),
                    });
                    (None, None, None)
                }
            };
        let asr_ok = writer.is_some();

        // Cleanup LLM (optional, CPU by default to keep GPU VRAM for ASR).
        let mut llama_guard = None;
        let cleaner: Option<LlamaCleaner> = if cfg.cleanup.enabled {
            match llama::LlamaServer::start(
                &cfg.cleanup.server,
                &cfg.cleanup.model,
                &cfg.cleanup.device,
                cfg.cleanup.port,
                cfg.cleanup.threads,
            ) {
                Ok(s) => {
                    // Keep the base prompt only; the dictionary is applied live per
                    // utterance (see `stop`) so editing it needs no restart. A custom
                    // system prompt from config overrides the built-in default.
                    let mut c = s.cleaner();
                    if !cfg.cleanup.prompt.trim().is_empty() {
                        c.system = cfg.cleanup.prompt.clone();
                    }
                    llama_guard = Some(s);
                    Some(c)
                }
                Err(e) => {
                    sink.emit(EngineEvent::Error {
                        message: format!("cleanup LLM failed to start ({e}); injecting raw"),
                    });
                    None
                }
            }
        } else {
            None
        };
        let cleanup_on = cleaner.is_some();

        // History store (optional).
        let history = if cfg.history.enabled {
            match glyph_core::History::open(&cfg.history.path) {
                Ok(h) => Some(Arc::new(h)),
                Err(e) => {
                    sink.emit(EngineEvent::Error { message: format!("history disabled: {e}") });
                    None
                }
            }
        } else {
            None
        };

        // Collector + finalizer: accumulate per-utterance ASR deltas, emit live
        // partials, and on Final run cleanup -> inject -> record -> Finalized.
        // Doing this here (not on the control thread) keeps the hotkey responsive
        // and delivers results strictly in arrival order — no blocking, no stale
        // results leaking into the next utterance.
        let sink_c = sink.clone();
        let collector = events.map(|events| thread::spawn(move || {
            let mut buf = String::new();
            for ev in events.iter() {
                buf.push_str(&ev.text);
                if ev.kind != EventKind::Final {
                    sink_c.emit(EngineEvent::Partial { text: buf.trim().to_string() });
                    continue;
                }
                let raw = buf.trim().to_string();
                buf.clear();
                if raw.is_empty() {
                    sink_c.emit(EngineEvent::Finalized {
                        raw: String::new(),
                        clean: String::new(),
                        app: String::new(),
                    });
                    continue;
                }
                // Snapshot live snippets + dictionary (don't hold the lock across
                // the cleanup network call).
                let (snippets, dict_terms) = {
                    let lv = live_ctl.lock().unwrap();
                    (lv.snippets.clone(), lv.dict_terms.clone())
                };
                let clean = match &cleaner {
                    Some(cl) => {
                        let mut cl = cl.clone();
                        if let Some(dict) = glyph_core::dictionary_prompt(&dict_terms) {
                            cl.system = format!("{}\n\n{}", cl.system, dict);
                        }
                        cl.clean(&raw)
                            .ok()
                            .filter(|c| !c.is_empty())
                            .unwrap_or_else(|| raw.clone())
                    }
                    None => raw.clone(),
                };
                let out = glyph_core::apply_snippets(&clean, &snippets);
                let (app, _) = inject::foreground_app();
                inject::inject(&out, method, keep_clip);
                if let Some(h) = &history {
                    h.record(&raw, &out, &app);
                }
                sink_c.emit(EngineEvent::Finalized { raw, clean: out, app });
            }
        }));

        // Hotkey hook (owns the Windows message loop). It reports its thread id so
        // the engine can post WM_QUIT to tear the hook down on a restart.
        let (hk_tx, hk_rx) = mpsc::channel::<HotkeyEvent>();
        let combo = cfg.hotkey.combo.clone();
        let sink_h = sink.clone();
        let hk_tid = Arc::new(AtomicU32::new(0));
        let hk_tid_c = hk_tid.clone();
        let hotkey = thread::spawn(move || {
            let res = hotkey::run(&combo, hk_tx, |tid| hk_tid_c.store(tid, Ordering::Relaxed));
            if let Err(e) = res {
                sink_h.emit(EngineEvent::Error { message: format!("hotkey: {e}") });
            }
        });

        // Control thread: react to hotkey events. It only opens/closes the mic and
        // marks the utterance boundary — finalization happens on the collector, so
        // this thread is always ready for the next press.
        let sink_ctl = sink.clone();
        let stop_ctl = stop.clone();
        let control = thread::spawn(move || {
            let mut active: Option<(CaptureStream, thread::JoinHandle<()>)> = None;

            let start = |active: &mut Option<(CaptureStream, thread::JoinHandle<()>)>| {
                if active.is_some() {
                    return;
                }
                let Some(w0) = writer.as_ref() else {
                    sink_ctl.emit(EngineEvent::Error {
                        message: "No transcription model loaded — pick one in Settings.".into(),
                    });
                    return;
                };
                match CaptureStream::start(Some(&mic)) {
                    Ok((cap, blocks)) => {
                        let rate = cap.device_rate;
                        let w = w0.clone();
                        let feeder = thread::spawn(move || feeder_loop(blocks, rate, w));
                        let (app, _) = inject::foreground_app();
                        sink_ctl.emit(EngineEvent::RecordingStarted { app });
                        *active = Some((cap, feeder));
                    }
                    Err(e) => sink_ctl.emit(EngineEvent::Error { message: format!("audio: {e}") }),
                }
            };

            let stop = |active: &mut Option<(CaptureStream, thread::JoinHandle<()>)>| {
                let Some((cap, feeder)) = active.take() else { return };
                drop(cap); // stop mic; closes the capture channel
                let _ = feeder.join(); // ensure all audio (+ tail) fed
                sink_ctl.emit(EngineEvent::Stopped);
                // Mark the utterance end; the collector emits Finalized when the
                // ASR result (and cleanup) come back.
                if let Some(w) = writer.as_ref() {
                    if w.end_utterance().is_err() {
                        sink_ctl.emit(EngineEvent::Error { message: "engine gone".into() });
                    }
                }
            };

            // Poll the hotkey channel with a timeout so a restart can break out
            // (the channel never disconnects on its own — its sender lives in the
            // static hook state).
            loop {
                // On shutdown just drop the capture (stops the mic); don't
                // end_utterance — the sidecar is already being told to quit, so a
                // failed write would emit a spurious "engine gone" error.
                if stop_ctl.load(Ordering::Relaxed) {
                    drop(active.take()); // stops the mic if recording
                    break;
                }
                let hk = match hk_rx.recv_timeout(Duration::from_millis(150)) {
                    Ok(hk) => hk,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        drop(active.take());
                        break;
                    }
                };
                let hold = hold_ctl.load(Ordering::Relaxed);
                match hk {
                    HotkeyEvent::Down => {
                        if hold {
                            start(&mut active);
                        } else if active.is_some() {
                            // Toggle, derived from whether we're recording — so it
                            // can never desync: a press stops if recording, else starts.
                            stop(&mut active);
                        } else {
                            start(&mut active);
                        }
                    }
                    HotkeyEvent::Up => {
                        if hold {
                            stop(&mut active);
                        }
                    }
                }
            }
        });

        if asr_ok {
            sink.emit(EngineEvent::Ready { cleanup: cleanup_on });
        } else {
            // Keep the badge informative (not stuck "Applying…") and the hotkey live.
            sink.emit(EngineEvent::Error {
                message: "No transcription model loaded — pick one in Settings.".into(),
            });
        }
        let mut threads = vec![hotkey, control];
        threads.extend(collector); // only present when ASR loaded
        Ok(Engine {
            _llama: llama_guard,
            shutdown: Some(Shutdown { writer: writer_sd, stop, hk_tid, threads }),
        })
    }
}

/// Pull raw device-rate mono blocks, resample to 16 kHz, feed the ASR. Flushes
/// the resampler tail when the capture channel closes.
fn feeder_loop(blocks: mpsc::Receiver<Vec<f32>>, device_rate: u32, w: StreamWriter) {
    let mut rs = match Resampler16k::new(device_rate) {
        Ok(rs) => rs,
        Err(e) => {
            eprintln!("[feeder] resampler: {e}");
            return;
        }
    };
    let mut peak = 0f32;
    let mut fed = 0usize;
    while let Ok(b) = blocks.recv() {
        for &s in &b {
            peak = peak.max(s.abs());
        }
        if let Ok(out) = rs.push(&b) {
            fed += out.len();
            let _ = w.feed(&out);
        }
    }
    if let Ok(tail) = rs.flush() {
        let _ = w.feed(&tail);
    }
    eprintln!(
        "[feeder] fed={fed} @16k peak={peak:.4}{}",
        if peak < 0.01 { "  <- near-silence; wrong/muted mic?" } else { "" }
    );
}
