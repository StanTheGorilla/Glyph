//! Post-ASR text features: personal dictionary + spoken snippets.

use std::collections::BTreeMap;

/// A cleanup-prompt addendum that biases the LLM to spell user terms correctly.
/// Returns None when there are no terms.
pub fn dictionary_prompt(terms: &[String]) -> Option<String> {
    let terms: Vec<&str> = terms.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if terms.is_empty() {
        return None;
    }
    Some(format!(
        "Known terms — keep these exact spellings/capitalization when they occur: {}.",
        terms.join(", ")
    ))
}

/// Expand spoken snippets: each cue phrase (case-insensitive) is replaced by its
/// expansion. Cues are expected to be ASCII phrases (e.g. "my email", "new line").
pub fn apply_snippets(text: &str, snippets: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (cue, expansion) in snippets {
        if !cue.trim().is_empty() {
            out = replace_ci(&out, cue, expansion);
        }
    }
    out
}

/// Case-insensitive replace-all. Assumes `needle` is ASCII (so lowercasing keeps
/// byte offsets aligned with the original string).
fn replace_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let lower_h = haystack.to_lowercase();
    let lower_n = needle.to_lowercase();
    let mut result = String::with_capacity(haystack.len());
    let mut last = 0;
    let mut search_from = 0;
    while let Some(rel) = lower_h[search_from..].find(&lower_n) {
        let abs = search_from + rel;
        result.push_str(&haystack[last..abs]);
        result.push_str(replacement);
        last = abs + needle.len();
        search_from = last;
    }
    result.push_str(&haystack[last..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_substitution_is_case_insensitive() {
        let mut s = BTreeMap::new();
        s.insert("my email".to_string(), "me@example.com".to_string());
        s.insert("new line".to_string(), "\n".to_string());
        let out = apply_snippets("send to My Email then New Line done", &s);
        assert_eq!(out, "send to me@example.com then \n done");
    }

    #[test]
    fn empty_snippets_noop() {
        let s = BTreeMap::new();
        assert_eq!(apply_snippets("hello", &s), "hello");
    }

    #[test]
    fn dictionary_prompt_terms() {
        assert!(dictionary_prompt(&[]).is_none());
        let p = dictionary_prompt(&["Glyph".into(), "Vulkan".into()]).unwrap();
        assert!(p.contains("Glyph") && p.contains("Vulkan"));
    }
}
