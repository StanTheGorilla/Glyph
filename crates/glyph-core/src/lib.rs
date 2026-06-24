//! glyph-core: the dictation engine traits and the parakeet sidecar client.
//!
//! `AsrEngine` is the backend-agnostic seam. The only implementation today is
//! `SidecarEngine`, which spawns `glyph-asr-sidecar` (a separate process wrapping
//! parakeet.dll) and talks to it: binary PCM frames in, newline-JSON events out.
//! A future in-process FFI engine can implement the same trait.

pub mod asr;
pub mod cleaner;
pub mod history;
pub mod proc_guard;
pub mod text;

pub use asr::{
    AsrEngine, EventKind, SidecarEngine, StreamConfig, StreamHandle, StreamWriter, TranscriptEvent,
};
pub use cleaner::{Cleaner, LlamaCleaner, DEFAULT_SYSTEM_PROMPT};
pub use history::{Entry as HistoryEntry, History};
pub use text::{apply_snippets, dictionary_prompt};
