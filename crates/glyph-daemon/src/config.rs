//! Daemon configuration, loaded from `glyph.toml` (created with defaults if absent).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: Hotkey,
    pub audio: Audio,
    pub asr: Asr,
    pub cleanup: Cleanup,
    pub inject: Inject,
    pub history: History,
    pub dictionary: Dictionary,
    /// Spoken cue -> expansion (applied after cleanup).
    pub snippets: BTreeMap<String, String>,
    pub paths: Paths,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Paths {
    /// Where the Backends tab downloads binaries/models. Empty = the app's
    /// per-user data dir (resolved by the Tauri layer, which knows that path).
    pub backends_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub enabled: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    /// Input device name substring; empty = system default.
    pub device: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Dictionary {
    /// Proper nouns / terms to keep spelled correctly (fed to the cleanup prompt).
    pub terms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleanup {
    /// If false, the raw transcript is injected (no LLM pass).
    pub enabled: bool,
    pub server: PathBuf,
    pub model: PathBuf,
    /// "cpu" (recommended — frees GPU VRAM for ASR) or "gpu".
    pub device: String,
    pub port: u16,
    pub threads: u32,
    /// Custom cleanup system prompt. Empty = the built-in default (DEFAULT_SYSTEM_PROMPT).
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkey {
    /// Combo string: a single key ("rctrl", "f8") or a chord ("ctrl+shift+space").
    pub combo: String,
    /// "hold" (push-to-talk) or "toggle".
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Asr {
    /// Which ASR backend to run: "nemotron" (parakeet streaming) or "whisper".
    pub kind: String,
    pub sidecar: PathBuf,
    pub dll: PathBuf,
    pub model: PathBuf,
    /// "auto" (first GPU), "Vulkan0", or "cpu".
    pub device: String,
    /// Locale prompt for nemotron: "en", "auto", ...
    pub lang: String,
    /// Whisper backend settings (used when `kind == "whisper"`).
    pub whisper: Whisper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Whisper {
    /// Path to the glyph-whisper-sidecar executable.
    pub binary: PathBuf,
    /// Model preset: "turbo", "medium", or "small". The matching ggml `.bin` is
    /// loaded from the same directory as the nemotron model (`asr.model`).
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Inject {
    /// "paste" (reliable primary) or "unicode" (fallback).
    pub method: String,
    /// Leave the dictated text on the clipboard after injecting (so it's always
    /// pasteable). If false, the previous clipboard contents are restored.
    pub keep_on_clipboard: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: Hotkey { combo: "rctrl".into(), mode: "hold".into() },
            audio: Audio { device: String::new() },
            asr: Asr {
                kind: "nemotron".into(),
                sidecar: default_sidecar(),
                dll: PathBuf::from(
                    r"F:\Glyph\phase0\lib-vulkan\parakeet-v0.3.2-lib-win-vulkan-x64\parakeet.dll",
                ),
                model: PathBuf::from(
                    r"F:\Glyph\phase0\models\nemotron-3.5-asr-streaming-0.6b-q8_0.gguf",
                ),
                device: "auto".into(),
                lang: "en".into(),
                whisper: Whisper {
                    binary: sidecar_path("glyph-whisper-sidecar"),
                    model: "turbo".into(),
                },
            },
            cleanup: Cleanup {
                enabled: true,
                server: PathBuf::from(r"F:\Glyph\phase0\llama\llama-server.exe"),
                model: PathBuf::from(r"F:\Glyph\phase0\models\Qwen3.5-0.8B-Q4_K_M.gguf"),
                device: "cpu".into(),
                port: 8077,
                threads: 8,
                prompt: String::new(),
            },
            inject: Inject { method: "paste".into(), keep_on_clipboard: true },
            history: History {
                enabled: true,
                path: PathBuf::from(r"F:\Glyph\glyph_history.db"),
            },
            dictionary: Dictionary::default(),
            snippets: BTreeMap::new(),
            paths: Paths::default(),
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Config::default().audio
    }
}

impl Default for History {
    fn default() -> Self {
        Config::default().history
    }
}

impl Default for Cleanup {
    fn default() -> Self {
        Config::default().cleanup
    }
}

impl Default for Hotkey {
    fn default() -> Self {
        Config::default().hotkey
    }
}
impl Default for Asr {
    fn default() -> Self {
        Config::default().asr
    }
}
impl Default for Whisper {
    fn default() -> Self {
        Config::default().asr.whisper
    }
}
impl Default for Inject {
    fn default() -> Self {
        Config::default().inject
    }
}

/// Resolve a sidecar executable that ships next to this app's exe — which is
/// where the installer places the bundled sidecars (Tauri `externalBin`).
fn sidecar_path(stem: &str) -> PathBuf {
    let file = if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join(&file);
        }
    }
    PathBuf::from(file)
}

fn default_sidecar() -> PathBuf {
    sidecar_path("glyph-asr-sidecar")
}

impl Config {
    /// Load from `path`, or write defaults there and return them if it doesn't exist.
    pub fn load_or_create(path: &Path) -> Result<Config> {
        if path.exists() {
            let s = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            toml::from_str(&s).with_context(|| format!("parse {}", path.display()))
        } else {
            let cfg = Config::default();
            let s = toml::to_string_pretty(&cfg)?;
            std::fs::write(path, s).with_context(|| format!("write {}", path.display()))?;
            eprintln!("[config] wrote defaults to {}", path.display());
            Ok(cfg)
        }
    }

    /// Resolve the active ASR sidecar exe and model path from `asr.kind`. The dll
    /// is the same for both (the whisper sidecar ignores `--dll`).
    pub fn active_asr(&self) -> (PathBuf, PathBuf) {
        if self.asr.kind.eq_ignore_ascii_case("whisper") {
            let file = match self.asr.whisper.model.as_str() {
                "medium" => "ggml-medium-q5_0.bin",
                "small" => "ggml-small-q5_1.bin",
                "large" => "ggml-large-v3-q5_0.bin",
                _ => "ggml-large-v3-turbo-q5_0.bin", // "turbo" / default
            };
            // Whisper ggml bins live alongside the nemotron model.
            let model = self
                .asr
                .model
                .parent()
                .map(|d| d.join(file))
                .unwrap_or_else(|| PathBuf::from(file));
            (self.asr.whisper.binary.clone(), model)
        } else {
            (self.asr.sidecar.clone(), self.asr.model.clone())
        }
    }

    pub fn device_arg(&self) -> Option<String> {
        match self.asr.device.to_lowercase().as_str() {
            "auto" | "" => None,
            other => Some(other.to_string()),
        }
    }

    /// Hotkey behavior: true = hold (push-to-talk), false = toggle (click/click).
    pub fn hold_mode(&self) -> bool {
        self.hotkey.mode.to_lowercase() != "toggle"
    }

    pub fn inject_method(&self) -> crate::inject::Method {
        match self.inject.method.to_lowercase().as_str() {
            "unicode" => crate::inject::Method::Unicode,
            _ => crate::inject::Method::Paste,
        }
    }
}
