//! LLM cleanup pass: turn raw dictation into clean written text.
//!
//! `Cleaner` is the trait; `LlamaCleaner` talks to any OpenAI-compatible
//! `/v1/chat/completions` endpoint (llama.cpp `llama-server`, Ollama, LM Studio).
//! Default model is a small instruct model run on CPU, so it stays fast and
//! leaves GPU VRAM for the ASR.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::json;

pub trait Cleaner {
    /// Clean a finalized raw transcript. Empty input returns empty.
    fn clean(&self, raw: &str) -> Result<String>;
}

/// Default cleanup system prompt: 2-shot, biased toward preserving wording.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You convert raw dictation (speech-to-text) into clean written text. Remove filler \
words (um, uh, like, you know, so), remove stuttered or repeated words, fix grammar, \
add punctuation, and capitalize sentences. Do NOT add new words, ideas, or pleasantries, \
and do not change the speaker's wording beyond the above. Output ONLY the cleaned text, \
nothing else.

Input: um i guess we could uh meet on on tuesday maybe
Output: I guess we could meet on Tuesday.

Input: so yeah the the thing is its kind of broken you know
Output: The thing is, it's kind of broken.";

#[derive(Clone)]
pub struct LlamaCleaner {
    /// Full chat-completions URL, e.g. http://127.0.0.1:8080/v1/chat/completions
    pub endpoint: String,
    pub model: String,
    pub system: String,
    pub temperature: f32,
    /// Disable Qwen3-style thinking via chat_template_kwargs.
    pub no_think: bool,
    pub timeout: Duration,
}

impl LlamaCleaner {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: "glyph-cleanup".into(),
            system: DEFAULT_SYSTEM_PROMPT.into(),
            temperature: 0.2,
            no_think: true,
            timeout: Duration::from_secs(20),
        }
    }
}

impl Cleaner for LlamaCleaner {
    fn clean(&self, raw: &str) -> Result<String> {
        if raw.trim().is_empty() {
            return Ok(String::new());
        }
        let mut body = json!({
            "model": self.model,
            "temperature": self.temperature,
            "max_tokens": 512,
            "messages": [
                {"role": "system", "content": self.system},
                {"role": "user", "content": raw},
            ],
        });
        if self.no_think {
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        }

        let resp = ureq::post(&self.endpoint)
            .timeout(self.timeout)
            .send_json(body)
            .map_err(|e| anyhow!("cleanup request failed: {e}"))?;
        let v: serde_json::Value = resp.into_json().context("parse cleanup response")?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("no content in cleanup response"))?;
        Ok(strip_think(content).trim().to_string())
    }
}

/// If the model emitted a `<think>…</think>` block anyway, keep only what follows.
fn strip_think(s: &str) -> &str {
    match s.rfind("</think>") {
        Some(i) => &s[i + "</think>".len()..],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::strip_think;
    #[test]
    fn strips_think_block() {
        assert_eq!(strip_think("<think>reasoning</think>\nHello there."), "\nHello there.");
        assert_eq!(strip_think("Just text."), "Just text.");
    }
}
