//! Glyph dictation engine library: hotkey -> capture -> ASR -> cleanup -> inject.
//! Used by the headless `glyph-daemon` bin and the Tauri app (same engine).

pub mod audio;
pub mod backends;
pub mod config;
pub mod engine;
pub mod hotkey;
pub mod inject;
pub mod llama;

pub use config::Config;
pub use engine::{Engine, EngineEvent, EventSink, LiveConfig};
