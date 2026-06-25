use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::{Emitter, Manager, PhysicalPosition, State, WindowEvent};
use tauri_plugin_opener::OpenerExt;

use glyph_core::{History, HistoryEntry};
use glyph_daemon::backends::{Archive, CatalogItem, ConfigTarget, DownloadOutcome, Variant};
use glyph_daemon::{Config, Engine, EngineEvent, EventSink, LiveConfig};

/// Shared config file (also used by the headless daemon during dev).
fn config_path() -> PathBuf {
    PathBuf::from(r"F:\Glyph\glyph.toml")
}

/// Kept in Tauri state: keeps the engine + cleanup server alive, plus a read
/// connection to the history DB.
struct AppState {
    _engine: Mutex<Option<Engine>>,
    history: Option<Arc<History>>,
    /// Latest engine readiness, so a frontend that mounts after the `Ready` event
    /// fired can still sync the current state (via the `engine_status` command).
    status: Arc<Mutex<EngineStatus>>,
    /// Live hotkey behavior shared with the engine's control thread: true = hold
    /// (push-to-talk), false = toggle (click/click). Flipping it takes effect on
    /// the next keypress. Owned here (not by the engine) so the settings UI can
    /// change it even while the engine is still loading.
    hold: Arc<AtomicBool>,
    /// Live snippets + dictionary, re-read by the engine on each utterance, so
    /// saving them applies without a restart. Owned here for the same reason.
    live: Arc<Mutex<LiveConfig>>,
    /// In-flight (and paused) downloads, keyed `id::variant`, so they can be
    /// paused, resumed, or cancelled from the UI.
    downloads: Mutex<HashMap<String, DlEntry>>,
    /// Serializes in-process engine restarts (model/engine/mic changes apply with
    /// no app relaunch) so two restarts can't interleave.
    restart: Mutex<()>,
}

/// Per-download controls. A paused entry is kept (with its `.part` path) so the
/// UI can resume it or cancel it (deleting the partial file).
struct DlEntry {
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    part: PathBuf,
}

#[derive(Default, Clone, serde::Serialize)]
struct EngineStatus {
    ready: bool,
    cleanup: bool,
    error: bool,
}

// ---- commands ----

#[tauri::command]
fn get_config() -> Result<Config, String> {
    Config::load_or_create(&config_path()).map_err(|e| e.to_string())
}

/// Save the config. Returns `true` if it triggered an engine restart (so the UI
/// can say "Applying…" vs just "Saved").
#[tauri::command]
fn save_config(app: tauri::AppHandle, state: State<AppState>, config: Config) -> Result<bool, String> {
    // Reject an unparseable hotkey here: otherwise the hook silently fails to
    // install on next launch and the hotkey appears to "do nothing".
    glyph_daemon::hotkey::parse_spec(&config.hotkey.combo)
        .map_err(|e| format!("invalid hotkey: {e}"))?;
    let old = Config::load_or_create(&config_path()).ok();
    // Download folders (`paths`) are owned by their own set_* commands, never by
    // this bulk auto-save — keep the on-disk values so a stale frontend copy
    // can't revert a folder the user just changed.
    let mut config = config;
    if let Some(o) = &old {
        config.paths = o.paths.clone();
    }
    let toml = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), toml).map_err(|e| e.to_string())?;
    // Mode, snippets and dictionary apply live (no restart).
    state.hold.store(config.hold_mode(), Ordering::Relaxed);
    *state.live.lock().unwrap() = LiveConfig {
        snippets: config.snippets.clone(),
        dict_terms: config.dictionary.terms.clone(),
    };
    // Engine/mic/hotkey/cleanup changes now apply by restarting the engine
    // in-process — no app relaunch.
    let restart = old.as_ref().map(|o| engine_fields_changed(o, &config)).unwrap_or(true);
    if restart {
        restart_engine_async(&app);
    }
    Ok(restart)
}

/// Apply the hold/toggle hotkey mode immediately (no Save / restart needed) and
/// persist just that field to disk so it survives relaunch. Called by the Mode
/// dropdown's change handler. `mode` is "hold" or "toggle".
#[tauri::command]
fn set_hotkey_mode(state: State<AppState>, mode: String) -> Result<(), String> {
    state.hold.store(mode.to_lowercase() != "toggle", Ordering::Relaxed);
    let mut cfg = Config::load_or_create(&config_path()).map_err(|e| e.to_string())?;
    cfg.hotkey.mode = mode;
    let toml = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), toml).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_config_raw() -> Result<String, String> {
    get_config().and_then(|c| toml::to_string_pretty(&c).map_err(|e| e.to_string()))
}

#[tauri::command]
fn save_config_raw(contents: String) -> Result<(), String> {
    toml::from_str::<Config>(&contents).map_err(|e| format!("invalid config: {e}"))?;
    std::fs::write(config_path(), contents).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_mics() -> Vec<String> {
    glyph_daemon::audio::input_device_names()
}

/// Current engine readiness. The frontend calls this on mount in case the engine
/// became ready before its event listener was attached.
#[tauri::command]
fn engine_status(state: State<AppState>) -> EngineStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
fn get_history(state: State<AppState>, limit: usize) -> Result<Vec<HistoryEntry>, String> {
    match &state.history {
        Some(h) => h.recent(limit).map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
fn clear_history(state: State<AppState>) -> Result<(), String> {
    if let Some(h) = &state.history {
        h.clear().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text).map_err(|e| e.to_string())
}

// ---- backends tab: download binaries/models ----

/// The directory the Backends tab downloads into: the user's override if set,
/// else the app's per-user data dir (`%LOCALAPPDATA%\com.glyph.app\backends`).
fn backends_root(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(cfg) = Config::load_or_create(&config_path()) {
        if !cfg.paths.backends_dir.as_os_str().is_empty() {
            return cfg.paths.backends_dir;
        }
    }
    app.path()
        .app_local_data_dir()
        .map(|d| d.join("backends"))
        .unwrap_or_else(|_| PathBuf::from("backends"))
}

/// Where transcription (ASR) models download: the user's override if set, else
/// the `asr` subfolder of the base backends dir.
fn transcription_models_root(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(cfg) = Config::load_or_create(&config_path()) {
        if !cfg.paths.transcription_dir.as_os_str().is_empty() {
            return cfg.paths.transcription_dir;
        }
    }
    backends_root(app).join("asr")
}

/// Where cleanup LLM models download: the user's override if set, else the
/// `cleanup` subfolder of the base backends dir.
fn cleanup_models_root(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(cfg) = Config::load_or_create(&config_path()) {
        if !cfg.paths.cleanup_dir.as_os_str().is_empty() {
            return cfg.paths.cleanup_dir;
        }
    }
    backends_root(app).join("cleanup")
}

/// The model directory for a custom-download `kind` ("transcription"/"cleanup").
fn models_root(app: &tauri::AppHandle, kind: &str) -> PathBuf {
    if kind == "transcription" {
        transcription_models_root(app)
    } else {
        cleanup_models_root(app)
    }
}

/// The directory a catalog item installs into. Transcription/cleanup models honor
/// their per-kind override dirs; engine binaries live under the base backends
/// dir's per-item subfolder.
fn item_root(app: &tauri::AppHandle, item: &CatalogItem) -> PathBuf {
    match item.subdir {
        "asr" => transcription_models_root(app),
        "cleanup" => cleanup_models_root(app),
        other => backends_root(app).join(other),
    }
}

#[tauri::command]
fn backend_catalog() -> Vec<CatalogItem> {
    glyph_daemon::backends::catalog()
}

#[tauri::command]
fn backends_dir(app: tauri::AppHandle) -> String {
    backends_root(&app).to_string_lossy().into_owned()
}

#[tauri::command]
fn set_backends_dir(path: String) -> Result<(), String> {
    let mut cfg = Config::load_or_create(&config_path()).map_err(|e| e.to_string())?;
    cfg.paths.backends_dir = PathBuf::from(path);
    let toml = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), toml).map_err(|e| e.to_string())
}

#[tauri::command]
fn transcription_dir(app: tauri::AppHandle) -> String {
    transcription_models_root(&app).to_string_lossy().into_owned()
}

#[tauri::command]
fn set_transcription_dir(path: String) -> Result<(), String> {
    let mut cfg = Config::load_or_create(&config_path()).map_err(|e| e.to_string())?;
    cfg.paths.transcription_dir = PathBuf::from(path);
    let toml = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), toml).map_err(|e| e.to_string())
}

#[tauri::command]
fn cleanup_dir(app: tauri::AppHandle) -> String {
    cleanup_models_root(&app).to_string_lossy().into_owned()
}

#[tauri::command]
fn set_cleanup_dir(path: String) -> Result<(), String> {
    let mut cfg = Config::load_or_create(&config_path()).map_err(|e| e.to_string())?;
    cfg.paths.cleanup_dir = PathBuf::from(path);
    let toml = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), toml).map_err(|e| e.to_string())
}

/// Registry key for an in-flight download (one per item+variant, so different
/// builds of the same item can download in parallel).
fn dl_key(id: &str, variant: &str) -> String {
    format!("{id}::{variant}")
}

#[derive(serde::Serialize)]
struct ItemStatus {
    id: String,
    /// Variant ids whose primary file is present on disk (can be several).
    installed_variants: Vec<String>,
    /// The variant the config currently points at (for items that set a config
    /// field); None for whisper (picked in Settings) or when nothing matches.
    active_variant: Option<String>,
}

/// The path a config target currently points at, if the target sets one.
fn config_target_path(cfg: &Config, target: ConfigTarget) -> Option<PathBuf> {
    match target {
        ConfigTarget::CleanupServer => Some(cfg.cleanup.server.clone()),
        ConfigTarget::CleanupModel => Some(cfg.cleanup.model.clone()),
        ConfigTarget::AsrModel => Some(cfg.asr.model.clone()),
        ConfigTarget::AsrDll => Some(cfg.asr.dll.clone()),
        ConfigTarget::None => None,
    }
}

/// For each catalog item, which variants are installed and which is active.
#[tauri::command]
fn backend_status(app: tauri::AppHandle) -> Vec<ItemStatus> {
    let cfg = Config::load_or_create(&config_path()).ok();
    glyph_daemon::backends::catalog()
        .iter()
        .map(|item| {
            let dir = item_root(&app, item);
            let installed_variants: Vec<String> = item
                .variants
                .iter()
                .filter(|v| item.primary_path(&dir, v).exists())
                .map(|v| v.id.to_string())
                .collect();
            let active_variant = cfg
                .as_ref()
                .and_then(|c| config_target_path(c, item.config_target))
                .and_then(|field| {
                    item.variants
                        .iter()
                        .find(|v| item.primary_path(&dir, v) == field)
                        .map(|v| v.id.to_string())
                });
            ItemStatus { id: item.id.to_string(), installed_variants, active_variant }
        })
        .collect()
}

/// Point the matching config field at an already-installed variant (the "Use"
/// action), so the user can switch between installed builds/models.
#[tauri::command]
fn activate_backend(app: tauri::AppHandle, id: String, variant: String) -> Result<(), String> {
    let catalog = glyph_daemon::backends::catalog();
    let item = catalog.iter().find(|i| i.id == id).ok_or("unknown item")?;
    let var = item.variant(&variant).ok_or("unknown variant")?;
    let primary = item.primary_path(&item_root(&app, item), var);
    if !primary.exists() {
        return Err("that build isn't installed".into());
    }
    wire_config(item.config_target, &primary).map_err(|e| e.to_string())?;
    restart_engine_async(&app);
    Ok(())
}

#[tauri::command]
fn hf_search(query: String, kind: String) -> Result<Vec<glyph_daemon::backends::HfModel>, String> {
    // Cleanup wants GGUF chat models; transcription drops the filter so whisper
    // `.bin` and other ASR repos surface too.
    let gguf_only = kind != "transcription";
    glyph_daemon::backends::hf_search(&query, gguf_only).map_err(|e| e.to_string())
}

#[tauri::command]
fn hf_gguf_files(repo: String, kind: String) -> Result<Vec<glyph_daemon::backends::HfFile>, String> {
    let exts: &[&str] = if kind == "transcription" { &[".gguf", ".bin"] } else { &[".gguf"] };
    glyph_daemon::backends::hf_model_files(&repo, exts).map_err(|e| e.to_string())
}

// ---- installed models (what's actually on disk, regardless of catalog) ----

#[derive(serde::Serialize)]
struct InstalledModel {
    /// "cleanup" or "transcription".
    kind: String,
    name: String,
    path: String,
    size: u64,
    /// True when the engine is currently configured to use this file.
    active: bool,
}

/// Collect model files of the given extensions from `dir` into `out`.
fn scan_models(dir: &Path, exts: &[&str], active: Option<&Path>, kind: &str, out: &mut Vec<InstalledModel>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext_ok = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(InstalledModel {
            kind: kind.to_string(),
            name,
            active: active.map(|a| a == p.as_path()).unwrap_or(false),
            path: p.to_string_lossy().into_owned(),
            size,
        });
    }
}

/// Add the active model to `out` if it isn't already listed (e.g. a bring-your-own
/// file that lives outside the managed folder), so any selected model stays
/// visible and re-selectable.
fn push_active_outside(out: &mut Vec<InstalledModel>, active: &Path, kind: &str) {
    if !active.is_file() || out.iter().any(|m| Path::new(&m.path) == active) {
        return;
    }
    let name = active.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let size = std::fs::metadata(active).map(|m| m.len()).unwrap_or(0);
    out.push(InstalledModel {
        kind: kind.to_string(),
        name,
        active: true,
        path: active.to_string_lossy().into_owned(),
        size,
    });
}

/// Every model file actually present in the managed folder, so the UI can show
/// what's installed (presets *and* custom downloads) and which is active.
#[tauri::command]
fn installed_models(app: tauri::AppHandle) -> Vec<InstalledModel> {
    let cfg = Config::load_or_create(&config_path()).ok();
    let mut out = Vec::new();
    let cleanup_active = cfg.as_ref().map(|c| c.cleanup.model.clone());
    scan_models(&cleanup_models_root(&app), &["gguf"], cleanup_active.as_deref(), "cleanup", &mut out);
    // active_asr() resolves the model the active engine actually loads (whisper bin
    // or nemotron gguf), so the right transcription file is flagged active.
    let asr_active = cfg.as_ref().map(|c| c.active_asr().1);
    scan_models(&transcription_models_root(&app), &["bin", "gguf"], asr_active.as_deref(), "transcription", &mut out);
    // Surface the active model even when it lives outside the managed folder (a
    // local/custom pick), so it doesn't silently vanish from the list.
    if let Some(p) = cleanup_active {
        push_active_outside(&mut out, &p, "cleanup");
    }
    if let Some(p) = asr_active {
        push_active_outside(&mut out, &p, "transcription");
    }
    out
}

/// Point the relevant config field(s) at `path` for the given model kind. For
/// transcription, a `.bin` is treated as a Whisper model (kind + size derived)
/// and a `.gguf` as the Nemotron model. Shared by activate/local/custom flows.
fn wire_model(kind: &str, path: &Path) -> anyhow::Result<()> {
    let mut cfg = Config::load_or_create(&config_path())?;
    match kind {
        "cleanup" => cfg.cleanup.model = path.to_path_buf(),
        "transcription" => {
            let lname = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            if lname.ends_with(".bin") {
                cfg.asr.kind = "whisper".into();
                // "turbo" before "large": the turbo bin's name also contains "large".
                cfg.asr.whisper.model = if lname.contains("medium") {
                    "medium"
                } else if lname.contains("small") {
                    "small"
                } else if lname.contains("turbo") {
                    "turbo"
                } else if lname.contains("large") {
                    "large"
                } else {
                    "turbo"
                }
                .into();
                cfg.asr.model = path.to_path_buf();
            } else {
                cfg.asr.kind = "nemotron".into();
                cfg.asr.model = path.to_path_buf();
            }
        }
        _ => anyhow::bail!("unknown model kind"),
    }
    let toml = toml::to_string_pretty(&cfg)?;
    std::fs::write(config_path(), toml)?;
    Ok(())
}

/// Switch the engine to an installed model (the "Use" action).
#[tauri::command]
fn activate_model(app: tauri::AppHandle, path: String, kind: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err("that model file is missing".into());
    }
    wire_model(&kind, &p).map_err(|e| e.to_string())?;
    restart_engine_async(&app);
    Ok(())
}

/// Delete a model file from the managed folder (the trash action in the model
/// list). Restricted to `.gguf`/`.bin` files inside the backends folder so it
/// can never be pointed at an arbitrary path.
#[tauri::command]
fn delete_model(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    let managed = [
        backends_root(&app),
        transcription_models_root(&app),
        cleanup_models_root(&app),
    ];
    if !managed.iter().any(|r| p.starts_with(r)) {
        return Err("that model isn't in the managed folder".into());
    }
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gguf") || e.eq_ignore_ascii_case("bin"))
        .unwrap_or(false);
    if !ext_ok {
        return Err("not a model file".into());
    }
    if !p.is_file() {
        return Err("that file is already gone".into());
    }
    std::fs::remove_file(&p).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_backends_dir(app: tauri::AppHandle) -> Result<(), String> {
    let root = backends_root(&app);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(root.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_transcription_dir(app: tauri::AppHandle) -> Result<(), String> {
    let root = transcription_models_root(&app);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(root.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_cleanup_dir(app: tauri::AppHandle) -> Result<(), String> {
    let root = cleanup_models_root(&app);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(root.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_download(state: State<AppState>, id: String, variant: String) {
    let key = dl_key(&id, &variant);
    let mut dl = state.downloads.lock().unwrap();
    if let Some(entry) = dl.get(&key) {
        if entry.pause.load(Ordering::Relaxed) {
            // Paused: no thread is running, so clean up the partial file ourselves
            // and drop the entry.
            let _ = std::fs::remove_file(&entry.part);
            dl.remove(&key);
        } else {
            // Active: the download thread sees the flag, deletes the .part, and exits.
            entry.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Pause an in-flight download: the thread stops but the `.part` is kept so
/// `download_backend`/`download_custom_model` can resume it later.
#[tauri::command]
fn pause_download(state: State<AppState>, id: String, variant: String) {
    if let Some(entry) = state.downloads.lock().unwrap().get(&dl_key(&id, &variant)) {
        entry.pause.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    id: String,
    variant: String,
    /// "download" | "extract" | "paused" | "done" | "error"
    phase: String,
    received: u64,
    total: u64,
    message: Option<String>,
}

fn emit_progress(
    app: &tauri::AppHandle,
    item: &CatalogItem,
    var: &Variant,
    phase: &str,
    received: u64,
    total: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            id: item.id.to_string(),
            variant: var.id.to_string(),
            phase: phase.to_string(),
            received,
            total,
            message,
        },
    );
}

/// A throttled progress callback (emits at most ~8×/sec, plus start/finish) so a
/// multi-hundred-MB download doesn't flood the webview with events.
fn progress_emitter(
    app: &tauri::AppHandle,
    item: &CatalogItem,
    var: &Variant,
) -> impl FnMut(u64, u64) {
    let app = app.clone();
    let item = item.clone();
    let var = var.clone();
    let mut last = std::time::Instant::now();
    move |received, total| {
        if received == 0
            || (total > 0 && received == total)
            || last.elapsed() >= std::time::Duration::from_millis(120)
        {
            last = std::time::Instant::now();
            emit_progress(&app, &item, &var, "download", received, total, None);
        }
    }
}

/// Start a backend download on a background thread. Progress and completion are
/// reported via `download-progress` events; this returns once the download is
/// queued (or immediately on a bad request).
#[tauri::command]
fn download_backend(
    app: tauri::AppHandle,
    state: State<AppState>,
    id: String,
    variant: String,
) -> Result<(), String> {
    let catalog = glyph_daemon::backends::catalog();
    let item = catalog.iter().find(|i| i.id == id).ok_or("unknown item")?.clone();
    let var = item.variant(&variant).ok_or("unknown variant")?.clone();
    if var.url.is_empty() {
        return Err("download source for this item isn't configured yet".into());
    }
    let root = item_root(&app, &item);
    let key = dl_key(&id, &variant);
    // The partial file kept across a pause (so a later cancel can remove it).
    let part = match var.archive {
        Archive::File => item.primary_path(&root, &var).with_extension("part"),
        Archive::Zip => item.install_dir(&root, &var).join("_download.part"),
    };

    // Register fresh controls, cancelling any prior active download for this build.
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock().unwrap();
        if let Some(old) = dl.get(&key) {
            old.cancel.store(true, Ordering::Relaxed);
        }
        dl.insert(key.clone(), DlEntry { cancel: cancel.clone(), pause: pause.clone(), part });
    }

    std::thread::spawn(move || {
        let res = run_download(&app, &root, &item, &var, &cancel, &pause);
        let paused = matches!(res, Ok(DownloadOutcome::Paused { .. }));
        // Keep the entry while paused (so it can be resumed/cancelled); otherwise
        // drop it if it's still ours.
        if !paused {
            if let Some(st) = app.try_state::<AppState>() {
                let mut dl = st.downloads.lock().unwrap();
                if dl.get(&key).map(|e| Arc::ptr_eq(&e.cancel, &cancel)).unwrap_or(false) {
                    dl.remove(&key);
                }
            }
        }
        match res {
            Ok(DownloadOutcome::Done) => {
                emit_progress(&app, &item, &var, "done", 0, 0, None);
                // Installing a server/model that wires a config field makes it
                // active — apply it by restarting the engine (no relaunch).
                if item.config_target != ConfigTarget::None {
                    restart_engine_async(&app);
                }
            }
            Ok(DownloadOutcome::Paused { received, total }) => {
                emit_progress(&app, &item, &var, "paused", received, total, None);
            }
            // A cancel already clears the row in the UI — don't also flash an error.
            Err(e) => {
                if !cancel.load(Ordering::Relaxed) {
                    emit_progress(&app, &item, &var, "error", 0, 0, Some(e.to_string()));
                }
            }
        }
    });
    Ok(())
}

/// Download (and extract, if a zip) a variant, then point the matching config
/// field at the installed file.
fn run_download(
    app: &tauri::AppHandle,
    root: &Path,
    item: &CatalogItem,
    var: &Variant,
    cancel: &AtomicBool,
    pause: &AtomicBool,
) -> anyhow::Result<DownloadOutcome> {
    use glyph_daemon::backends::{download_file, extract_zip};

    let install_dir = item.install_dir(root, var);
    std::fs::create_dir_all(&install_dir)?;

    match var.archive {
        Archive::File => {
            let dest = install_dir.join(var.primary);
            if let DownloadOutcome::Paused { received, total } =
                download_file(var.url, &dest, cancel, pause, progress_emitter(app, item, var))?
            {
                return Ok(DownloadOutcome::Paused { received, total });
            }
        }
        Archive::Zip => {
            let zip_path = install_dir.join("_download.zip");
            if let DownloadOutcome::Paused { received, total } =
                download_file(var.url, &zip_path, cancel, pause, progress_emitter(app, item, var))?
            {
                return Ok(DownloadOutcome::Paused { received, total });
            }
            emit_progress(app, item, var, "extract", 0, 0, None);
            extract_zip(&zip_path, &install_dir)?;
            let _ = std::fs::remove_file(&zip_path);
            // A second archive shipped alongside (e.g. CUDA's cudart DLLs).
            if let Some(extra) = var.extra_url {
                let extra_zip = install_dir.join("_extra.zip");
                if let DownloadOutcome::Paused { received, total } =
                    download_file(extra, &extra_zip, cancel, pause, progress_emitter(app, item, var))?
                {
                    return Ok(DownloadOutcome::Paused { received, total });
                }
                emit_progress(app, item, var, "extract", 0, 0, None);
                extract_zip(&extra_zip, &install_dir)?;
                let _ = std::fs::remove_file(&extra_zip);
            }
        }
    }

    wire_config(item.config_target, &item.primary_path(root, var))?;
    Ok(DownloadOutcome::Done)
}

/// Persist the installed file's path into `glyph.toml` so the engine uses it on
/// next launch. `None` items (whisper) are resolved from `asr.model`'s dir instead.
fn wire_config(target: ConfigTarget, primary: &Path) -> anyhow::Result<()> {
    if target == ConfigTarget::None {
        return Ok(());
    }
    let mut cfg = Config::load_or_create(&config_path())?;
    let p = primary.to_path_buf();
    match target {
        ConfigTarget::CleanupServer => cfg.cleanup.server = p,
        ConfigTarget::CleanupModel => cfg.cleanup.model = p,
        ConfigTarget::AsrModel => cfg.asr.model = p,
        ConfigTarget::AsrDll => cfg.asr.dll = p,
        ConfigTarget::None => {}
    }
    let toml = toml::to_string_pretty(&cfg)?;
    std::fs::write(config_path(), toml)?;
    Ok(())
}

// ---- custom cleanup model (bring-your-own GGUF) ----

/// HF "blob" view URLs serve an HTML page; rewrite to the raw "resolve" URL so the
/// actual file downloads.
fn normalize_hf_url(url: &str) -> String {
    url.replace("/blob/", "/resolve/")
}

/// The trailing model filename of a URL (query/fragment stripped), if it ends in
/// an accepted extension (`.gguf`, plus `.bin` for transcription).
fn model_filename_from_url(url: &str, kind: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next()?.to_string();
    let lower = name.to_lowercase();
    let ok = lower.ends_with(".gguf") || (kind == "transcription" && lower.ends_with(".bin"));
    (name.len() > 5 && ok).then_some(name)
}

fn emit_custom(
    app: &tauri::AppHandle,
    kind: &str,
    phase: &str,
    received: u64,
    total: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            id: "custom-model".to_string(),
            // variant = kind, so the cleanup and transcription custom rows track
            // independently (keys "custom-model::cleanup" / "::transcription").
            variant: kind.to_string(),
            phase: phase.to_string(),
            received,
            total,
            message,
        },
    );
}

/// Download an arbitrary model from a direct link into the kind's folder
/// (`cleanup`/`asr`) and point the right config field at it. Progress is reported
/// under id "custom-model", variant `kind`.
#[tauri::command]
fn download_custom_model(
    app: tauri::AppHandle,
    state: State<AppState>,
    url: String,
    kind: String,
) -> Result<(), String> {
    let url = normalize_hf_url(url.trim());
    let filename = model_filename_from_url(&url, &kind)
        .ok_or("that link doesn't point at a model file — use a direct .gguf or .bin link")?;
    let dest = models_root(&app, &kind).join(filename);
    let key = dl_key("custom-model", &kind);
    let part = dest.with_extension("part");

    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock().unwrap();
        if let Some(old) = dl.get(&key) {
            old.cancel.store(true, Ordering::Relaxed);
        }
        dl.insert(key.clone(), DlEntry { cancel: cancel.clone(), pause: pause.clone(), part });
    }

    std::thread::spawn(move || {
        let res = run_custom_download(&app, &url, &dest, &kind, &cancel, &pause);
        let paused = matches!(res, Ok(DownloadOutcome::Paused { .. }));
        if !paused {
            if let Some(st) = app.try_state::<AppState>() {
                let mut dl = st.downloads.lock().unwrap();
                if dl.get(&key).map(|e| Arc::ptr_eq(&e.cancel, &cancel)).unwrap_or(false) {
                    dl.remove(&key);
                }
            }
        }
        match res {
            Ok(DownloadOutcome::Done) => {
                emit_custom(&app, &kind, "done", 0, 0, None);
                // The custom model is now the active one — apply without relaunch.
                restart_engine_async(&app);
            }
            Ok(DownloadOutcome::Paused { received, total }) => {
                emit_custom(&app, &kind, "paused", received, total, None);
            }
            Err(e) => {
                if !cancel.load(Ordering::Relaxed) {
                    emit_custom(&app, &kind, "error", 0, 0, Some(e.to_string()));
                }
            }
        }
    });
    Ok(())
}

fn run_custom_download(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    kind: &str,
    cancel: &AtomicBool,
    pause: &AtomicBool,
) -> anyhow::Result<DownloadOutcome> {
    let mut last = std::time::Instant::now();
    let outcome = glyph_daemon::backends::download_file(url, dest, cancel, pause, |received, total| {
        if received == 0
            || (total > 0 && received == total)
            || last.elapsed() >= std::time::Duration::from_millis(120)
        {
            last = std::time::Instant::now();
            emit_custom(app, kind, "download", received, total, None);
        }
    })?;
    if matches!(outcome, DownloadOutcome::Paused { .. }) {
        return Ok(outcome);
    }
    wire_model(kind, dest)?;
    Ok(DownloadOutcome::Done)
}

/// Use a model file that already exists on disk for the given kind (no download).
#[tauri::command]
fn use_local_model(app: tauri::AppHandle, path: String, kind: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err("that path isn't a file".into());
    }
    wire_model(&kind, &p).map_err(|e| e.to_string())?;
    restart_engine_async(&app);
    Ok(())
}

/// The currently-configured cleanup model path (shown in the Backends tab).
#[tauri::command]
fn cleanup_model_path() -> String {
    Config::load_or_create(&config_path())
        .map(|c| c.cleanup.model.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The built-in cleanup system prompt, so the UI can show it and offer "reset".
#[tauri::command]
fn default_cleanup_prompt() -> String {
    glyph_core::DEFAULT_SYSTEM_PROMPT.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            set_hotkey_mode,
            get_config_raw,
            save_config_raw,
            list_mics,
            engine_status,
            get_history,
            clear_history,
            copy_text,
            backend_catalog,
            backends_dir,
            set_backends_dir,
            transcription_dir,
            set_transcription_dir,
            cleanup_dir,
            set_cleanup_dir,
            backend_status,
            download_backend,
            cancel_download,
            pause_download,
            reveal_backends_dir,
            reveal_transcription_dir,
            reveal_cleanup_dir,
            activate_backend,
            hf_search,
            hf_gguf_files,
            installed_models,
            activate_model,
            delete_model,
            download_custom_model,
            use_local_model,
            cleanup_model_path,
            default_cleanup_prompt,
        ])
        .setup(|app| {
            // Ensure the llama-server + ASR sidecars die with Glyph (even on a
            // Task Manager kill) instead of lingering in the background. Arms the
            // kill-job before any child spawns; each child is also added to it.
            glyph_core::proc_guard::init();
            setup_tray(app.handle())?;
            keep_main_in_tray(app.handle());
            show_hud_overlay(app.handle());

            let cfg = Config::load_or_create(&config_path())?;
            let history = if cfg.history.enabled {
                History::open(&cfg.history.path).ok().map(Arc::new)
            } else {
                None
            };
            let status = Arc::new(Mutex::new(EngineStatus::default()));
            let hold = Arc::new(AtomicBool::new(cfg.hold_mode()));
            let live = Arc::new(Mutex::new(LiveConfig {
                snippets: cfg.snippets.clone(),
                dict_terms: cfg.dictionary.terms.clone(),
            }));
            app.manage(AppState {
                _engine: Mutex::new(None),
                history,
                status: status.clone(),
                hold: hold.clone(),
                live: live.clone(),
                downloads: Mutex::new(HashMap::new()),
                restart: Mutex::new(()),
            });

            // Load the engine off the main thread so the window appears instantly
            // instead of freezing during model load, and so the `Ready` event is
            // emitted after the event loop is running (otherwise the UI never
            // receives it and is stuck showing "starting…").
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let engine = start_engine(&handle, cfg, status, hold, live);
                *handle.state::<AppState>()._engine.lock().unwrap() = engine;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// True if a config change touches fields the engine reads only at startup (ASR,
/// mic, cleanup, injection, hotkey combo) — i.e. it needs an engine restart to
/// apply. Snippets, dictionary and hotkey mode apply live, so they're excluded.
fn engine_fields_changed(old: &Config, new: &Config) -> bool {
    fn s<T: serde::Serialize>(v: &T) -> String {
        toml::to_string(v).unwrap_or_default()
    }
    s(&old.asr) != s(&new.asr)
        || old.audio.device != new.audio.device
        || s(&old.cleanup) != s(&new.cleanup)
        || s(&old.inject) != s(&new.inject)
        || old.hotkey.combo != new.hotkey.combo
}

/// Stop the running engine and start a fresh one from the current on-disk config,
/// off the main thread. This is what lets a model/engine/mic change apply without
/// relaunching the app. Emits `engine-reload` so the UI can show "Applying…".
fn restart_engine_async(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let state = handle.state::<AppState>();
        let _serialize = state.restart.lock().unwrap();
        let _ = handle.emit("engine-reload", ());
        {
            let mut s = state.status.lock().unwrap();
            *s = EngineStatus::default();
        }
        // Drop the old engine first: its Drop kills the cleanup server (freeing the
        // port), tells the ASR sidecar to quit, and uninstalls the keyboard hook —
        // all of which must happen before the new engine claims them.
        *state._engine.lock().unwrap() = None;
        let cfg = match Config::load_or_create(&config_path()) {
            Ok(c) => c,
            Err(e) => {
                let _ = handle.emit(
                    "engine-event",
                    EngineEvent::Error { message: format!("reload failed: {e}") },
                );
                return;
            }
        };
        let engine = start_engine(&handle, cfg, state.status.clone(), state.hold.clone(), state.live.clone());
        *state._engine.lock().unwrap() = engine;
    });
}

/// Start the dictation engine; forward events to the HUD.
///
/// The HUD is a persistent overlay (shown once in `setup`, see `show_hud_overlay`),
/// so the engine only forwards events here — it must NOT show/hide the window.
/// Showing a fresh top-most window while a fullscreen app is focused is exactly
/// what kicks the user out of fullscreen; keeping the window persistent (and
/// `WS_EX_NOACTIVATE`, applied on startup) avoids that entirely.
fn start_engine(
    handle: &tauri::AppHandle,
    cfg: Config,
    status: Arc<Mutex<EngineStatus>>,
    hold: Arc<AtomicBool>,
    live: Arc<Mutex<LiveConfig>>,
) -> Option<Engine> {
    let h = handle.clone();
    let status_c = status.clone();
    let sink: Arc<dyn EventSink> = Arc::new(move |ev: EngineEvent| {
        match &ev {
            EngineEvent::Ready { cleanup } => {
                let mut s = status_c.lock().unwrap();
                s.ready = true;
                s.cleanup = *cleanup;
                s.error = false;
            }
            EngineEvent::Error { .. } => {
                let mut s = status_c.lock().unwrap();
                s.ready = true;
                s.error = true;
            }
            _ => {}
        }
        let _ = h.emit("engine-event", ev);
    });

    match Engine::start(cfg, sink, hold, live) {
        Ok(engine) => Some(engine),
        Err(e) => {
            {
                let mut s = status.lock().unwrap();
                s.ready = true;
                s.error = true;
            }
            let _ = handle.emit(
                "engine-event",
                EngineEvent::Error { message: format!("engine failed to start: {e}") },
            );
            eprintln!("engine start failed: {e}");
            None
        }
    }
}

/// Position the HUD as a half-disc on the upper-right edge of the primary
/// monitor, then make it a persistent always-on-top overlay and show it once. It is never shown/hidden
/// again by the engine — only its color animates — which is what keeps it from
/// exiting fullscreen apps.
///
/// `WS_EX_NOACTIVATE` is the key style: a window that can never receive
/// activation does not steal focus and therefore does not cause the focused
/// fullscreen application (game/video) to minimize/exit. `WS_EX_TRANSPARENT`
/// makes it click-through, `WS_EX_LAYERED` enables per-pixel transparency, and
/// `WS_EX_TOPMOST` keeps it above fullscreen windows.
fn show_hud_overlay(handle: &tauri::AppHandle) {
    let Some(hud) = handle.get_webview_window("hud") else { return };

    // Position bottom-center on the primary monitor.
    if let Ok(Some(mon)) = hud.primary_monitor() {
        let sz = mon.size();
        let scale = mon.scale_factor();
        let w = 120.0 * scale;
        let h = 120.0 * scale;
        // Half-disc hugging the right edge: the orb (centered in its window) is
        // placed with its center just past the screen edge, so its right portion
        // clips off and a clear half-circle shows. This keeps the actual top-right
        // corner — the window close buttons — clear, while staying upper-right.
        // The small inward nudge guards against the invisible window-frame margin
        // pushing the disc too far off-screen.
        let x = sz.width as f64 - w / 2.0 - 8.0 * scale;
        let y = 140.0 * scale - h / 2.0;
        let _ = hud.set_position(PhysicalPosition::new(x, y));
    }

    apply_overlay_styles(&hud);
    let _ = hud.show();
}

/// Apply the Win32 extended styles that make a window a proper fullscreen-safe
/// overlay, and that keep tiling window managers (komorebi, GlazeWM, FancyZones)
/// from managing it. On non-Windows this is a no-op (the Tauri config flags
/// approximate it).
#[cfg(windows)]
fn apply_overlay_styles(window: &tauri::WebviewWindow) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_EX_WINDOWEDGE,
    };

    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = HWND(h.hwnd.get() as *mut core::ffi::c_void);
    // SAFETY: reading/writing the extended style of our own window. These ex-styles
    // are additive and safe to combine.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        // Add the overlay ex-styles, then strip the two that mark a window as a
        // "normal app window" so tiling window managers leave the HUD alone:
        // komorebi only tiles windows that have CAPTION+WINDOWEDGE and aren't
        // layered, and GlazeWM/FancyZones skip tool windows / non-APPWINDOW windows.
        let add = (WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0
            | WS_EX_TOOLWINDOW.0) as isize;
        let remove = (WS_EX_WINDOWEDGE.0 | WS_EX_APPWINDOW.0) as isize;
        let new_ex = (ex | add) & !remove;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex);
        // Commit the style change so it takes effect immediately and external
        // observers re-read it, without moving, resizing, or activating the window.
        let _ = SetWindowPos(
            hwnd,
            HWND(std::ptr::null_mut()),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
fn apply_overlay_styles(_window: &tauri::WebviewWindow) {}

fn setup_tray(handle: &tauri::AppHandle) -> tauri::Result<()> {
    let settings = MenuItemBuilder::with_id("settings", "Open Settings").build(handle)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Glyph").build(handle)?;
    let menu = MenuBuilder::new(handle).items(&[&settings, &quit]).build()?;
    if let Some(tray) = handle.tray_by_id("glyph-tray") {
        tray.set_menu(Some(menu))?;
        tray.on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => {
                // Drop the engine now so its sidecars (llama-server, ASR) are
                // killed before we leave: app.exit() calls process::exit, which
                // skips destructors, so we can't rely on Engine's Drop here.
                if let Some(st) = app.try_state::<AppState>() {
                    *st._engine.lock().unwrap() = None;
                }
                app.exit(0);
            }
            _ => {}
        });
    }
    Ok(())
}

/// Closing the settings window hides it (app keeps running in the tray).
fn keep_main_in_tray(handle: &tauri::AppHandle) {
    if let Some(main) = handle.get_webview_window("main") {
        let main2 = main.clone();
        main.on_window_event(move |ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = main2.hide();
            }
        });
    }
}
