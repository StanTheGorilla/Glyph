//! Downloadable backends/models catalog + download/extract helpers for the
//! Backends tab.
//!
//! This is pure logic (no Tauri types): the app layer (`glyph-app`) wraps these,
//! resolves the download directory, emits progress events, and writes the
//! resolved path back into `glyph.toml`. Streaming reuses the existing `ureq`
//! dependency; `zip` is used to unpack the binary archives.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Which `Config` path field an item populates once installed.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigTarget {
    /// `cleanup.server`
    CleanupServer,
    /// `cleanup.model`
    CleanupModel,
    /// `asr.model` (the nemotron gguf)
    AsrModel,
    /// `asr.dll` (parakeet runtime)
    AsrDll,
    /// Whisper ggml `.bin` — resolved by `Config::active_asr` from `asr.model`'s
    /// directory, so installing one sets no field (it just has to land in `asr/`).
    None,
}

/// How a variant's download is packaged.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Archive {
    /// Direct file download saved as `primary`.
    File,
    /// Zip extracted into the variant's install dir; `primary` is the file the
    /// config should point at.
    Zip,
}

/// One downloadable choice for an item: a GPU build (cpu/vulkan/cuda) or a model
/// size/quant. Empty `url` = source not yet pinned (UI disables install).
#[derive(Debug, Clone, Serialize)]
pub struct Variant {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub url: &'static str,
    /// Secondary archive fetched into the same dir (e.g. CUDA cudart DLLs).
    pub extra_url: Option<&'static str>,
    /// Human-readable size for display (the real total comes from the download).
    pub size: &'static str,
    pub archive: Archive,
    /// File the config points at: the saved filename (`File`) or the file inside
    /// the extracted folder (`Zip`).
    pub primary: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogItem {
    pub id: &'static str,
    pub group: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Sub-directory under the download root that holds this item.
    pub subdir: &'static str,
    pub config_target: ConfigTarget,
    pub variants: Vec<Variant>,
}

impl CatalogItem {
    pub fn variant(&self, id: &str) -> Option<&Variant> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// Directory (under `root`) the variant installs into. Zips get their own
    /// per-variant folder; direct files share the item's subdir.
    pub fn install_dir(&self, root: &Path, v: &Variant) -> PathBuf {
        match v.archive {
            Archive::Zip => root.join(self.subdir).join(v.id),
            Archive::File => root.join(self.subdir),
        }
    }

    /// Full path of the primary file — both the config target and the marker
    /// used to decide whether the variant is installed.
    pub fn primary_path(&self, root: &Path, v: &Variant) -> PathBuf {
        self.install_dir(root, v).join(v.primary)
    }
}

/// The full catalog. URLs for llama.cpp, Whisper and the cleanup LLM are pinned;
/// the parakeet/nemotron sources are left empty pending confirmation of the exact
/// `parakeet.cpp` release (the mechanism is identical once a URL is filled in).
pub fn catalog() -> Vec<CatalogItem> {
    vec![
        CatalogItem {
            id: "llama-server",
            group: "Cleanup",
            name: "llama.cpp server",
            description: "Runs the cleanup LLM locally (llama-server). Pick the build that matches your hardware.",
            subdir: "llama",
            config_target: ConfigTarget::CleanupServer,
            variants: vec![
                Variant {
                    id: "cpu",
                    label: "CPU",
                    note: "works everywhere",
                    url: "https://github.com/ggml-org/llama.cpp/releases/download/b9747/llama-b9747-bin-win-cpu-x64.zip",
                    extra_url: None,
                    size: "~40 MB",
                    archive: Archive::Zip,
                    primary: "llama-server.exe",
                },
                Variant {
                    id: "vulkan",
                    label: "Vulkan",
                    note: "most GPUs",
                    url: "https://github.com/ggml-org/llama.cpp/releases/download/b9747/llama-b9747-bin-win-vulkan-x64.zip",
                    extra_url: None,
                    size: "~50 MB",
                    archive: Archive::Zip,
                    primary: "llama-server.exe",
                },
                Variant {
                    id: "cuda",
                    label: "CUDA",
                    note: "NVIDIA only",
                    url: "https://github.com/ggml-org/llama.cpp/releases/download/b9747/llama-b9747-bin-win-cuda-12.4-x64.zip",
                    extra_url: Some("https://github.com/ggml-org/llama.cpp/releases/download/b9747/cudart-llama-bin-win-cuda-12.4-x64.zip"),
                    size: "~500 MB",
                    archive: Archive::Zip,
                    primary: "llama-server.exe",
                },
            ],
        },
        CatalogItem {
            id: "cleanup-model",
            group: "Cleanup",
            name: "Cleanup LLM",
            description: "The instruct model the cleanup pass runs. Pick a preset by how much your hardware can handle — or use a custom GGUF below.",
            subdir: "cleanup",
            config_target: ConfigTarget::CleanupModel,
            variants: vec![
                Variant {
                    id: "qwen3.5-0.8b",
                    label: "Qwen3.5 0.8B",
                    note: "fast · light",
                    url: "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/Qwen3.5-0.8B-Q4_K_M.gguf",
                    extra_url: None,
                    size: "~560 MB",
                    archive: Archive::File,
                    primary: "Qwen3.5-0.8B-Q4_K_M.gguf",
                },
                Variant {
                    id: "gemma-e2b",
                    label: "Gemma 4 E2B",
                    note: "more capable",
                    url: "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/main/gemma-4-E2B_q4_0-it.gguf",
                    extra_url: None,
                    size: "~1.6 GB",
                    archive: Archive::File,
                    primary: "gemma-4-E2B_q4_0-it.gguf",
                },
            ],
        },
        CatalogItem {
            id: "whisper-model",
            group: "Transcription",
            name: "Whisper model",
            description: "Speech-to-text model for the Whisper backend. Bigger = more accurate, slower.",
            subdir: "asr",
            config_target: ConfigTarget::None,
            variants: vec![
                Variant {
                    id: "turbo",
                    label: "Turbo",
                    note: "large-v3 · fast + accurate",
                    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
                    extra_url: None,
                    size: "574 MB",
                    archive: Archive::File,
                    primary: "ggml-large-v3-turbo-q5_0.bin",
                },
                Variant {
                    id: "medium",
                    label: "Medium",
                    note: "balanced",
                    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin",
                    extra_url: None,
                    size: "539 MB",
                    archive: Archive::File,
                    primary: "ggml-medium-q5_0.bin",
                },
                Variant {
                    id: "small",
                    label: "Small",
                    note: "lightest",
                    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
                    extra_url: None,
                    size: "190 MB",
                    archive: Archive::File,
                    primary: "ggml-small-q5_1.bin",
                },
                Variant {
                    id: "large",
                    label: "Large",
                    note: "large-v3 · most accurate",
                    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin",
                    extra_url: None,
                    size: "~1.1 GB",
                    archive: Archive::File,
                    primary: "ggml-large-v3-q5_0.bin",
                },
            ],
        },
        CatalogItem {
            id: "asr-engine",
            group: "Transcription",
            name: "Transcription engine (parakeet)",
            description: "Runtime for the Nemotron streaming backend (parakeet.dll). Whisper uses a built-in engine and needs no download. Pick the build that matches your hardware.",
            subdir: "parakeet",
            config_target: ConfigTarget::AsrDll,
            variants: vec![
                // The parakeet release zips wrap their files in a versioned
                // top-level folder (unlike llama.cpp's flat zips), so `primary`
                // includes that folder — that's where parakeet.dll actually lands.
                Variant {
                    id: "cpu",
                    label: "CPU",
                    note: "works everywhere",
                    url: "https://github.com/mudler/parakeet.cpp/releases/download/v0.3.2/parakeet-v0.3.2-lib-win-cpu-x64.zip",
                    extra_url: None,
                    size: "~15 MB",
                    archive: Archive::Zip,
                    primary: "parakeet-v0.3.2-lib-win-cpu-x64/parakeet.dll",
                },
                Variant {
                    id: "vulkan",
                    label: "Vulkan",
                    note: "most GPUs",
                    url: "https://github.com/mudler/parakeet.cpp/releases/download/v0.3.2/parakeet-v0.3.2-lib-win-vulkan-x64.zip",
                    extra_url: None,
                    size: "~17 MB",
                    archive: Archive::Zip,
                    primary: "parakeet-v0.3.2-lib-win-vulkan-x64/parakeet.dll",
                },
                Variant {
                    id: "cuda",
                    label: "CUDA",
                    note: "NVIDIA only",
                    url: "https://github.com/mudler/parakeet.cpp/releases/download/v0.3.2/parakeet-v0.3.2-lib-win-cuda-x64.zip",
                    extra_url: Some("https://github.com/mudler/parakeet.cpp/releases/download/v0.3.2/cudart-parakeet-bin-win-cuda-x64.zip"),
                    size: "~250 MB",
                    archive: Archive::Zip,
                    primary: "parakeet-v0.3.2-lib-win-cuda-x64/parakeet.dll",
                },
            ],
        },
    ]
}

/// How a download ended: fully finished, or paused by the user (the `.part` file
/// is left on disk so it can be resumed later).
pub enum DownloadOutcome {
    Done,
    Paused { received: u64, total: u64 },
}

/// Stream `url` to `dest`, reporting `(received, total)`. `cancel` aborts and
/// deletes the partial file; `pause` stops but keeps the `.part` for resuming. A
/// pre-existing `.part` is resumed via an HTTP Range request (falling back to a
/// fresh download if the server ignores it).
pub fn download_file(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    pause: &AtomicBool,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<DownloadOutcome> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = dest.with_extension("part");
    let have = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);

    let mut req = ureq::get(url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| anyhow!("request failed: {e}"))?;
    let resuming = have > 0 && resp.status() == 206;

    // When resuming, the full size is the trailing part of `Content-Range:
    // bytes a-b/total`; otherwise it's `Content-Length`.
    let (mut file, mut received, total) = if resuming {
        let total = resp
            .header("Content-Range")
            .and_then(|h| h.rsplit('/').next())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let file = fs::OpenOptions::new()
            .append(true)
            .open(&tmp)
            .with_context(|| format!("open {}", tmp.display()))?;
        (file, have, total)
    } else {
        let total = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
        let file = fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        (file, 0u64, total)
    };

    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    on_progress(received, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = fs::remove_file(&tmp);
            bail!("cancelled");
        }
        if pause.load(Ordering::Relaxed) {
            file.flush().ok();
            drop(file); // keep the .part on disk for a later resume
            return Ok(DownloadOutcome::Paused { received, total });
        }
        let n = reader.read(&mut buf).context("read response")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).context("write to disk")?;
        received += n as u64;
        on_progress(received, total);
    }
    file.flush().ok();
    drop(file);
    // Guard against a truncated download when the server told us the size.
    if total > 0 && received != total {
        let _ = fs::remove_file(&tmp);
        bail!("incomplete download: got {received} of {total} bytes");
    }
    fs::rename(&tmp, dest).with_context(|| format!("finalize {}", dest.display()))?;
    Ok(DownloadOutcome::Done)
}

// ---- Hugging Face model search (for the custom-model search bar) ----

/// A model repo returned by HF search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfModel {
    pub id: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
}

/// A `.gguf` file inside a model repo.
#[derive(Debug, Clone, Serialize)]
pub struct HfFile {
    pub path: String,
    pub size: u64,
}

/// Percent-encode a query string for use in a URL.
fn url_encode(q: &str) -> String {
    let mut s = String::with_capacity(q.len());
    for b in q.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => s.push(b as char),
            b' ' => s.push_str("%20"),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

/// Search Hugging Face for models, most-downloaded first. `gguf_only` applies the
/// `gguf` library filter (good for cleanup LLMs); transcription search leaves it
/// off so whisper.cpp (`.bin`) and other ASR repos surface too.
pub fn hf_search(query: &str, gguf_only: bool) -> Result<Vec<HfModel>> {
    let filter = if gguf_only { "&filter=gguf" } else { "" };
    let url = format!(
        "https://huggingface.co/api/models?search={}{}&sort=downloads&direction=-1&limit=15",
        url_encode(query.trim()),
        filter,
    );
    let resp = ureq::get(&url).call().map_err(|e| anyhow!("Hugging Face search failed: {e}"))?;
    let models: Vec<HfModel> = resp.into_json().context("parse HF search response")?;
    Ok(models)
}

/// List the model files (smallest first) in a HF model repo matching any of
/// `exts` (e.g. `[".gguf"]` or `[".gguf", ".bin"]`), skipping mmproj vision
/// projectors which aren't usable here.
pub fn hf_model_files(repo: &str, exts: &[&str]) -> Result<Vec<HfFile>> {
    #[derive(Deserialize)]
    struct TreeEntry {
        #[serde(rename = "type")]
        kind: String,
        path: String,
        #[serde(default)]
        size: u64,
    }
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
    let resp = ureq::get(&url).call().map_err(|e| anyhow!("Hugging Face file list failed: {e}"))?;
    let entries: Vec<TreeEntry> = resp.into_json().context("parse HF tree response")?;
    let mut files: Vec<HfFile> = entries
        .into_iter()
        .filter(|e| {
            let p = e.path.to_lowercase();
            e.kind == "file" && exts.iter().any(|x| p.ends_with(x)) && !p.contains("mmproj")
        })
        .map(|e| HfFile { path: e.path, size: e.size })
        .collect();
    files.sort_by_key(|f| f.size);
    Ok(files)
}

/// Extract every entry of `zip_path` into `dest_dir` (zip-slip safe).
pub fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("read zip archive")?;
    fs::create_dir_all(dest_dir).with_context(|| format!("create {}", dest_dir.display()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("read zip entry")?;
        // enclosed_name() rejects absolute paths and `..` traversal.
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest_dir.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).ok();
        } else {
            if let Some(p) = out.parent() {
                fs::create_dir_all(p).ok();
            }
            let mut outf =
                fs::File::create(&out).with_context(|| format!("create {}", out.display()))?;
            std::io::copy(&mut entry, &mut outf).context("extract file")?;
        }
    }
    Ok(())
}
