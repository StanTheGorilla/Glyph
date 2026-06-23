//! Headless Glyph daemon: hold the hotkey, speak into any app, cleaned text is
//! injected at the cursor. The GUI (Tauri) drives the same `Engine`.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;

use glyph_daemon::config::Config;
use glyph_daemon::hotkey::HotkeyEvent;
use glyph_daemon::{audio, hotkey, inject, Engine, EngineEvent, EventSink, LiveConfig};

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("probe-audio") => audio::probe(),
        Some("probe-audio-raw") => audio::probe_raw(),
        Some("mics") => audio::list_mics(),
        Some("test-hotkey") => test_hotkey(),
        Some("test-inject") => test_inject(),
        _ => run_daemon(),
    }
}

fn run_daemon() -> Result<()> {
    let cfg = Config::load_or_create(Path::new("glyph.toml"))?;
    println!(
        "Glyph daemon — hotkey [{}] ({}), model {}",
        cfg.hotkey.combo,
        cfg.hotkey.mode,
        cfg.asr.model.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()
    );
    println!("starting engine (model load + Vulkan warmup)…");

    let sink: Arc<dyn EventSink> = Arc::new(|ev: EngineEvent| match ev {
        EngineEvent::Ready { cleanup } => {
            println!("engine ready (cleanup: {}). Hold the hotkey and speak.", if cleanup { "on" } else { "off" })
        }
        EngineEvent::RecordingStarted { app } => println!("● recording…  (focus: {app})"),
        EngineEvent::Partial { .. } => {} // quiet in console
        EngineEvent::Stopped => {}
        EngineEvent::Finalized { raw, clean, .. } => {
            if raw.is_empty() {
                eprintln!("· (nothing captured)");
            } else if raw == clean {
                println!("✓ {clean}");
            } else {
                println!("✓ raw:   {raw}");
                println!("✦ clean: {clean}");
            }
        }
        EngineEvent::Error { message } => eprintln!("[engine] {message}"),
    });

    let hold = Arc::new(AtomicBool::new(cfg.hold_mode()));
    let live = Arc::new(Mutex::new(LiveConfig {
        snippets: cfg.snippets.clone(),
        dict_terms: cfg.dictionary.terms.clone(),
    }));
    let _engine = Engine::start(cfg, sink, hold, live)?;
    // Engine runs on background threads; keep the process alive.
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

/// Verify injection leaves the text on the clipboard.
fn test_inject() -> Result<()> {
    let text = "Glyph clipboard test ✓";
    println!("injecting {text:?} (paste, keep-on-clipboard)…");
    inject::inject(text, inject::Method::Paste, true);
    thread::sleep(Duration::from_millis(200));
    let mut cb = arboard::Clipboard::new()?;
    let got = cb.get_text().unwrap_or_default();
    println!("clipboard now: {got:?}\nPASS={}", got == text);
    Ok(())
}

/// Print hotkey Down/Up events live — run this and press your combo to verify it.
fn test_hotkey() -> Result<()> {
    let cfg = Config::load_or_create(Path::new("glyph.toml"))?;
    println!("press/hold [{}] (Ctrl+C to quit)", cfg.hotkey.combo);
    let (tx, rx) = mpsc::channel::<HotkeyEvent>();
    let combo = cfg.hotkey.combo.clone();
    thread::spawn(move || {
        let _ = hotkey::run(&combo, tx, |_tid| {});
    });
    for ev in rx {
        match ev {
            HotkeyEvent::Down => println!("DOWN (recording would start)"),
            HotkeyEvent::Up => println!("UP   (would finalize + inject)"),
        }
    }
    Ok(())
}
