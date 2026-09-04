use anyhow::{anyhow, Result};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use tracing::{info, warn};

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct OmpSessionStats {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub total_tokens: u64,
    pub steps_taken: u32,
}

pub struct OmpRunner;

impl OmpRunner {
    pub fn run_agent(
        model: &str,
        prompt: &str,
        workdir: &Path,
        api_key: Option<&str>,
        max_turns: u32,
    ) -> Result<OmpSessionStats> {
        Self::run_agent_with_logger(model, prompt, workdir, api_key, max_turns, |line| {
            println!("{}", line);
        })
    }

    pub fn run_agent_with_logger<F>(
        model: &str,
        prompt: &str,
        workdir: &Path,
        api_key: Option<&str>,
        max_turns: u32,
        mut log_fn: F,
    ) -> Result<OmpSessionStats>
    where
        F: FnMut(String) + Send + 'static,
    {
        info!("Launching OMP agent with model: {}", model);

        let mut cmd = Command::new("omp");
        cmd.arg("--approval-mode=yolo")
            .arg("-p")
            .arg(prompt)
            .arg(format!("--model={}", model))
            .arg(format!("--cwd={}", workdir.display()))
            .env("PI_NO_PTY", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(key) = api_key {
            cmd.env("OPENROUTER_API_KEY", key);
        }

        info!("Executing OMP command in: {}", workdir.display());
        let mut child = cmd.spawn().map_err(|e| anyhow!("Failed to spawn omp: {}", e))?;

        let (tx, rx) = mpsc::channel();

        // Spawn thread to read stdout
        if let Some(stdout) = child.stdout.take() {
            let tx_out = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    let _ = tx_out.send(line);
                }
            });
        }

        // Spawn thread to read stderr
        if let Some(stderr) = child.stderr.take() {
            let tx_err = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let _ = tx_err.send(line);
                }
            });
        }
        drop(tx);

        for line in rx {
            log_fn(line);
        }

        let status = child.wait().map_err(|e| anyhow!("Failed to wait for omp: {}", e))?;
        if !status.success() {
            warn!("OMP agent exited with non-zero status: {:?}", status.code());
        }

        let stats = Self::extract_latest_session_stats().unwrap_or_else(|e| {
            warn!("Could not read session stats: {}, using estimation", e);
            OmpSessionStats {
                prompt_tokens: 15_000,
                completion_tokens: 2_500,
                cached_tokens: 10_000,
                total_tokens: 17_500,
                steps_taken: max_turns,
            }
        });

        Ok(stats)
    }

    fn extract_latest_session_stats() -> Result<OmpSessionStats> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/ubuntu".to_string());
        let sessions_dir = Path::new(&home).join(".omp/agent/sessions");
        if !sessions_dir.exists() {
            return Err(anyhow!("Sessions dir does not exist"));
        }

        let mut latest_file = None;
        let mut latest_time = std::time::SystemTime::UNIX_EPOCH;

        if let Ok(entries) = std::fs::read_dir(sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let session_jsonl = path.join("session.jsonl");
                    if session_jsonl.exists() {
                        if let Ok(meta) = session_jsonl.metadata() {
                            if let Ok(modified) = meta.modified() {
                                if modified > latest_time {
                                    latest_time = modified;
                                    latest_file = Some(session_jsonl);
                                }
                            }
                        }
                    }
                }
            }
        }

        let session_file = latest_file.ok_or_else(|| anyhow!("No session file found"))?;
        let content = std::fs::read_to_string(session_file)?;

        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;
        let mut cached_tokens = 0u64;
        let mut steps = 0u32;

        for line in content.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                steps += 1;
                if let Some(usage) = v.get("usage") {
                    if let Some(pt) = usage.get("prompt_tokens").and_then(|x| x.as_u64()) {
                        prompt_tokens = prompt_tokens.max(pt);
                    }
                    if let Some(ct) = usage.get("completion_tokens").and_then(|x| x.as_u64()) {
                        completion_tokens += ct;
                    }
                    if let Some(details) = usage.get("prompt_tokens_details") {
                        if let Some(c) = details.get("cached_tokens").and_then(|x| x.as_u64()) {
                            cached_tokens = cached_tokens.max(c);
                        }
                    }
                    if let Some(c) = usage.get("cache_read_input_tokens").and_then(|x| x.as_u64()) {
                        cached_tokens = cached_tokens.max(c);
                    }
                }
            }
        }

        Ok(OmpSessionStats {
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            steps_taken: steps,
        })
    }
}
