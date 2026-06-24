//! llama.cpp `llama-server` sidecar lifecycle for the cleanup LLM.
//! Spawns the server (CPU by default, to leave GPU VRAM for ASR), waits for it
//! to become healthy, hands out a `LlamaCleaner`, and kills it on drop.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use glyph_core::LlamaCleaner;

pub struct LlamaServer {
    child: Child,
    base: String,
}

impl LlamaServer {
    pub fn start(server: &Path, model: &Path, device: &str, port: u16, threads: u32) -> Result<Self> {
        let ngl = if device.eq_ignore_ascii_case("cpu") { "0" } else { "99" };
        let mut cmd = Command::new(server);
        cmd.args(["-m"]).arg(model);
        cmd.args([
            "--host", "127.0.0.1",
            "--port", &port.to_string(),
            "-ngl", ngl,
            "-c", "2048",
            "--no-webui",
            "--jinja",
            "-t", &threads.to_string(),
        ]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: don't pop a console window when spawned by the GUI app.
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let child = cmd.spawn().context("spawn llama-server")?;
        // Tie llama-server to the kill-job so it dies with us even on a Task Manager
        // kill or a Quit that calls process::exit (Drop wouldn't run then).
        glyph_core::proc_guard::guard(&child);

        let base = format!("http://127.0.0.1:{port}");
        wait_healthy(&base, Duration::from_secs(90))?;
        Ok(Self { child, base })
    }

    pub fn cleaner(&self) -> LlamaCleaner {
        LlamaCleaner::new(format!("{}/v1/chat/completions", self.base))
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_healthy(base: &str, timeout: Duration) -> Result<()> {
    let url = format!("{base}/health");
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(2)).call() {
            if let Ok(v) = resp.into_json::<serde_json::Value>() {
                if v["status"].as_str() == Some("ok") {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(anyhow!("llama-server at {base} did not become healthy in time"))
}
