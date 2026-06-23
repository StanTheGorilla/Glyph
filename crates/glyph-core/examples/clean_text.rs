//! Verify the Rust LlamaCleaner against a running OpenAI-compatible server.
//! Start a server first, then: cargo run -p glyph-core --example clean_text -- "raw text"
//! Endpoint via GLYPH_LLAMA (default http://127.0.0.1:8077/v1/chat/completions).

use glyph_core::{Cleaner, LlamaCleaner};

fn main() -> anyhow::Result<()> {
    let endpoint = std::env::var("GLYPH_LLAMA")
        .unwrap_or_else(|_| "http://127.0.0.1:8077/v1/chat/completions".into());
    let cleaner = LlamaCleaner::new(endpoint);

    let raw = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "um so like i was thinking we should probably refactor the the auth module before friday".into());

    let out = cleaner.clean(&raw)?;
    println!("RAW:   {raw}");
    println!("CLEAN: {out}");
    assert!(!out.is_empty(), "cleaner returned empty");
    Ok(())
}
