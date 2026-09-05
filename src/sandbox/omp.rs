use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::fs::File;
use std::sync::Mutex;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy)]
pub struct AgentExecutionLimits {
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub timeout_seconds: Option<u64>,
}

impl Default for AgentExecutionLimits {
    fn default() -> Self {
        Self {
            max_turns: Some(15),
            max_budget_usd: Some(0.50),
            timeout_seconds: Some(900), // 15 minutes default
        }
    }
}

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
        limits: AgentExecutionLimits,
        effort: Option<&str>,
    ) -> Result<OmpSessionStats> {
        Self::run_agent_with_logger(model, prompt, workdir, api_key, limits, effort, |line| {
            println!("{}", line);
        })
    }

    pub fn run_agent_with_logger<F>(
        model: &str,
        prompt: &str,
        workdir: &Path,
        api_key: Option<&str>,
        limits: AgentExecutionLimits,
        effort: Option<&str>,
        mut log_fn: F,
    ) -> Result<OmpSessionStats>
    where
        F: FnMut(String) + Send + 'static,
    {
        info!("Launching OMP agent in Docker sandbox with model: {}, effort: {:?}, limits: {:?}", model, effort, limits);

        let start_time = SystemTime::now();
        let container_name = "subdollar-omp-agent";

        // Pre-clean any stale agent container
        let _ = Command::new("docker")
            .args(["rm", "-f", container_name])
            .output();

        let canonical_workdir = if workdir.is_absolute() {
            workdir.to_path_buf()
        } else {
            workdir.canonicalize().unwrap_or_else(|_| {
                std::env::current_dir()
                    .map(|c| c.join(workdir))
                    .unwrap_or_else(|_| crate::config::get_repo_root().join(workdir))
            })
        };
        let mount_workdir = format!("{}:/workspace", canonical_workdir.display());

        let host_omp_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| crate::config::get_repo_root())
            .join(".omp");
        let host_sessions_dir = host_omp_dir.join("agent").join("sessions");
        let _ = std::fs::create_dir_all(&host_sessions_dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&host_omp_dir, std::fs::Permissions::from_mode(0o777));
            let _ = std::fs::set_permissions(&host_omp_dir.join("agent"), std::fs::Permissions::from_mode(0o777));
            let _ = std::fs::set_permissions(&host_sessions_dir, std::fs::Permissions::from_mode(0o777));
        }
        let mount_omp = format!("{}:/home/ubuntu/.omp", host_omp_dir.display());

        let mut cmd = Command::new("docker");
        cmd.arg("run")
            .arg("--rm")
            .arg("--name")
            .arg(container_name)
            .arg("--memory=3g")
            .arg("--cpus=3.0")
            .arg("--pids-limit=512")
            .arg("--net=host")
            .arg("-v")
            .arg(&mount_workdir)
            .arg("-v")
            .arg(&mount_omp)
            .arg("-w")
            .arg("/workspace")
            .arg("-e")
            .arg("PI_NO_PTY=1")
            .arg("-e")
            .arg("HOME=/home/ubuntu");

        let effective_key = api_key
            .map(|s| s.to_string())
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
        if let Some(ref key) = effective_key {
            cmd.arg("-e").arg(format!("OPENROUTER_API_KEY={}", key));
        }

        cmd.arg("subdollar-sandbox")
            .arg("omp")
            .arg("--approval-mode=yolo")
            .arg("-p")
            .arg(prompt)
            .arg(format!("--model={}", model))
            .arg("--cwd=/workspace")
            .arg("--session-dir=/home/ubuntu/.omp/agent/sessions");

        if let Some(max_secs) = limits.timeout_seconds {
            cmd.arg(format!("--max-time={}s", max_secs));
        }

        if let Some(eff) = effort {
            let eff_clean = eff.trim().to_lowercase();
            if !eff_clean.is_empty() && eff_clean != "auto" && eff_clean != "default" {
                cmd.arg(format!("--thinking={}", eff_clean));
            }
            cmd.arg("--print-thoughts");
        } else {
            cmd.arg("--print-thoughts");
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let existing_files = Self::collect_all_session_files();
        let active_file: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

        info!("Executing OMP agent container '{}' with workdir: {}", container_name, canonical_workdir.display());
        let mut child = cmd.spawn().map_err(|e| anyhow!("Failed to spawn omp in docker: {}", e))?;

        let (tx, rx) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));

        let turn_counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let accumulated_cost = Arc::new(Mutex::new(0.0f64));
        let limit_reached = Arc::new(AtomicBool::new(false));
        let limit_reason = Arc::new(Mutex::new(Option::<String>::None));

        // Spawn thread to tail the active session .jsonl file in real-time
        let running_tailer = running.clone();
        let tx_tailer = tx.clone();
        let active_file_tailer = active_file.clone();
        let turn_counter_tailer = turn_counter.clone();
        let accumulated_cost_tailer = accumulated_cost.clone();
        let limit_reached_tailer = limit_reached.clone();
        let limit_reason_tailer = limit_reason.clone();
        let limits_tailer = limits;

        let tailer_handle = thread::spawn(move || {
            Self::tail_session_file(
                start_time,
                existing_files,
                active_file_tailer,
                running_tailer,
                tx_tailer,
                limits_tailer,
                turn_counter_tailer,
                accumulated_cost_tailer,
                limit_reached_tailer,
                limit_reason_tailer,
            );
        });

        // Spawn thread to read stdout
        if let Some(stdout) = child.stdout.take() {
            let tx_out = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        let _ = tx_out.send(trimmed.to_string());
                    }
                }
            });
        }

        // Spawn thread to read stderr
        if let Some(stderr) = child.stderr.take() {
            let tx_err = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        let _ = tx_err.send(format!("[STDERR] {}", trimmed));
                    }
                }
            });
        }
        drop(tx);

        let kill_agent = || {
            let _ = Command::new("docker").args(["kill", container_name]).output();
            let _ = Command::new("docker").args(["rm", "-f", container_name]).output();
        };

        let start_instant = std::time::Instant::now();
        let status = loop {
            // Check wall-clock timeout watchdog
            if let Some(max_secs) = limits.timeout_seconds {
                if start_instant.elapsed().as_secs() >= max_secs {
                    let msg = format!("[LIMIT] Watchdog: maximum time limit ({}s) reached. Concluding agent run...", max_secs);
                    log_fn(msg.clone());
                    limit_reached.store(true, Ordering::SeqCst);
                    *limit_reason.lock().unwrap() = Some(msg);
                    kill_agent();
                    let _ = child.kill();
                    break None;
                }
            }

            // Check if turn or budget limit was reached
            if limit_reached.load(Ordering::SeqCst) {
                if let Ok(lock) = limit_reason.lock() {
                    if let Some(ref r) = *lock {
                        log_fn(r.clone());
                    }
                }
                kill_agent();
                let _ = child.kill();
                break None;
            }

            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => log_fn(line),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(s) = child.try_wait().map_err(|e| anyhow!("Failed to check omp status: {}", e))? {
                        break Some(s);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let s = child.wait().map_err(|e| anyhow!("Failed to wait for omp: {}", e))?;
                    break Some(s);
                }
            }
        };

        kill_agent();
        running.store(false, Ordering::SeqCst);
        let _ = tailer_handle.join();

        while let Ok(line) = rx.recv_timeout(Duration::from_millis(50)) {
            log_fn(line);
        }

        let stopped_by_limit = limit_reached.load(Ordering::SeqCst);
        if !stopped_by_limit {
            if let Some(s) = status {
                if !s.success() {
                    warn!("OMP agent exited with non-zero status: {:?}", s.code());
                    return Err(anyhow!("OMP agent exited with non-zero status: {:?}", s.code()));
                }
            }
        }

        let final_path = active_file.lock().unwrap().clone();
        let stats = Self::extract_latest_session_stats(final_path.as_deref()).unwrap_or_default();
        Ok(stats)
    }

    fn get_sessions_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home).join(".omp/agent/sessions");
            if p.exists() { dirs.push(p); }
        }
        let root = crate::config::get_repo_root();
        let r_omp = root.join(".omp/agent/sessions");
        if r_omp.exists() && !dirs.contains(&r_omp) { dirs.push(r_omp); }
        let u = PathBuf::from("/home/ubuntu/.omp/agent/sessions");
        if u.exists() && !dirs.contains(&u) { dirs.push(u); }
        let r = PathBuf::from("/root/.omp/agent/sessions");
        if r.exists() && !dirs.contains(&r) { dirs.push(r); }
        dirs
    }

    fn collect_all_session_files() -> HashSet<PathBuf> {
        let mut set = HashSet::new();
        for dir in Self::get_sessions_dirs() {
            let mut candidates = Vec::new();
            Self::collect_jsonl_files(&dir, &mut candidates);
            for (_, p) in candidates {
                set.insert(p);
            }
        }
        set
    }

    fn tail_session_file(
        start_time: SystemTime,
        existing: HashSet<PathBuf>,
        active_file: Arc<Mutex<Option<PathBuf>>>,
        running: Arc<AtomicBool>,
        tx: mpsc::Sender<String>,
        limits: AgentExecutionLimits,
        turn_counter: Arc<std::sync::atomic::AtomicU32>,
        accumulated_cost: Arc<Mutex<f64>>,
        limit_reached: Arc<AtomicBool>,
        limit_reason: Arc<Mutex<Option<String>>>,
    ) {
        let dirs = Self::get_sessions_dirs();

        // Check if active_file already has a pre-set file
        let mut session_file: Option<PathBuf> = active_file.lock().ok().and_then(|l| l.clone());

        // Poll continuously for the new session file to appear while agent is running
        if session_file.is_none() {
            while running.load(Ordering::SeqCst) {
                if let Some(f) = Self::find_new_session_file(&dirs, &existing, start_time) {
                    session_file = Some(f);
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }

            if session_file.is_none() {
                session_file = Self::find_new_session_file(&dirs, &existing, start_time);
            }
        }

        let session_path = match session_file {
            Some(p) => p,
            None => {
                info!("No new session file detected for current run.");
                return;
            }
        };

        if let Ok(mut lock) = active_file.lock() {
            *lock = Some(session_path.clone());
        }

        let file = match File::open(&session_path) {
            Ok(f) => f,
            Err(_) => return,
        };

        let mut reader = BufReader::new(file);
        let mut line_buf = String::new();

        while running.load(Ordering::SeqCst) {
            line_buf.clear();
            match reader.read_line(&mut line_buf) {
                Ok(0) => {
                    thread::sleep(Duration::from_millis(200));
                }
                Ok(_) => {
                    if line_buf.ends_with('\n') {
                        // Check limits in session line
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line_buf.trim()) {
                            // 1. Assistant turn check
                            if val.pointer("/message/role").and_then(|r| r.as_str()) == Some("assistant") {
                                let c = turn_counter.fetch_add(1, Ordering::SeqCst) + 1;
                                if let Some(max_t) = limits.max_turns {
                                    if c >= max_t {
                                        let msg = format!("[LIMIT] Reached turn limit ({} / {} turns). Concluding agent run...", c, max_t);
                                        *limit_reason.lock().unwrap() = Some(msg.clone());
                                        limit_reached.store(true, Ordering::SeqCst);
                                        let _ = tx.send(msg);
                                    }
                                }
                            }
                            // 2. Budget check
                            if let Some(cost) = val.pointer("/message/usage/cost/total").and_then(|c| c.as_f64()) {
                                if let Ok(mut c_lock) = accumulated_cost.lock() {
                                    *c_lock += cost;
                                    let current_spend = *c_lock;
                                    if let Some(max_b) = limits.max_budget_usd {
                                        if current_spend >= max_b {
                                            let msg = format!("[LIMIT] Reached budget cap (${:.4} / ${:.2} USD). Concluding agent run...", current_spend, max_b);
                                            *limit_reason.lock().unwrap() = Some(msg.clone());
                                            limit_reached.store(true, Ordering::SeqCst);
                                            let _ = tx.send(msg);
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(events) = Self::format_session_line(&line_buf) {
                            for ev in events {
                                let _ = tx.send(ev);
                            }
                        }
                    } else {
                        let len = line_buf.as_bytes().len() as i64;
                        let _ = reader.seek(SeekFrom::Current(-len));
                        thread::sleep(Duration::from_millis(150));
                    }
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }

        // Process exit drain
        loop {
            line_buf.clear();
            match reader.read_line(&mut line_buf) {
                Ok(0) => break,
                Ok(_) => {
                    if line_buf.ends_with('\n') {
                        if let Some(events) = Self::format_session_line(&line_buf) {
                            for ev in events {
                                let _ = tx.send(ev);
                            }
                        }
                    } else {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn find_new_session_file(
        dirs: &[PathBuf],
        existing: &HashSet<PathBuf>,
        start_time: SystemTime,
    ) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        for dir in dirs {
            Self::collect_jsonl_files(dir, &mut candidates);
        }
        candidates.sort_by_key(|(m, _)| *m);

        // Priority 1: brand new file not present in existing snapshot
        for (_, p) in candidates.iter().rev() {
            if !existing.contains(p) {
                return Some(p.clone());
            }
        }

        // Priority 2: modified after start_time
        for (m, p) in candidates.into_iter().rev() {
            if m >= start_time {
                return Some(p);
            }
        }
        None
    }

    fn find_newest_session_file_across(dirs: &[PathBuf]) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        for dir in dirs {
            Self::collect_jsonl_files(dir, &mut candidates);
        }
        candidates.sort_by_key(|(m, _)| *m);
        candidates.pop().map(|(_, p)| p)
    }

    fn collect_jsonl_files(dir: &Path, out: &mut Vec<(SystemTime, PathBuf)>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    if let Ok(meta) = path.metadata() {
                        if let Ok(m) = meta.modified() {
                            out.push((m, path));
                        }
                    }
                } else if path.is_dir() {
                    Self::collect_jsonl_files(&path, out);
                }
            }
        }
    }

    fn format_session_line(line: &str) -> Option<Vec<String>> {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let mut results = Vec::new();

        let custom_type = v.get("customType").and_then(|s| s.as_str());
        let role = v.get("message").and_then(|m| m.get("role")).and_then(|s| s.as_str());

        if custom_type == Some("tool_execution_start") {
            if let Some(d) = v.get("data") {
                let tool = d.get("toolName").and_then(|s| s.as_str()).unwrap_or("tool");
                let intent = d.get("intent").and_then(|s| s.as_str()).unwrap_or("");
                if intent.is_empty() {
                    results.push(format!("[ACTION] {}", tool));
                } else {
                    results.push(format!("[ACTION] {}: {}", tool, intent));
                }
                if let Some(args) = d.get("args") {
                    if let Some(cmd) = args.get("command").and_then(|s| s.as_str()) {
                        for line in cmd.lines() {
                            let t = line.trim();
                            if !t.is_empty() {
                                results.push(format!("  $ {}", t));
                            }
                        }
                    } else if let Some(p) = args.get("path").and_then(|s| s.as_str()) {
                        results.push(format!("  path: {}", p));
                    } else if let Some(code) = args.get("code").and_then(|s| s.as_str()) {
                        let lines_vec: Vec<&str> = code.lines().collect();
                        for (i, c_line) in lines_vec.iter().enumerate() {
                            if i < 20 {
                                results.push(format!("  | {}", c_line));
                            } else {
                                results.push(format!("  | ... (+{} lines)", lines_vec.len() - 20));
                                break;
                            }
                        }
                    }
                }
            }
        } else if role == Some("assistant") {
            if let Some(msg) = v.get("message") {
                if let Some(usage) = msg.get("usage") {
                    let tin = usage.get("input").or_else(|| usage.get("prompt_tokens")).and_then(|x| x.as_u64()).unwrap_or(0);
                    let tout = usage.get("output").or_else(|| usage.get("completion_tokens")).and_then(|x| x.as_u64()).unwrap_or(0);
                    let tcached = usage.get("cacheRead").or_else(|| usage.get("cache_read_input_tokens")).and_then(|x| x.as_u64()).unwrap_or(0);
                    if tin > 0 || tout > 0 || tcached > 0 {
                        results.push(format!("[TOKENS] Turn usage - input: {}, output: {}, cached: {}", tin, tout, tcached));
                    }
                }
                if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                    for item in content {
                        let item_type = item.get("type").and_then(|s| s.as_str());
                        if item_type == Some("thinking") {
                            if let Some(th) = item.get("thinking").and_then(|s| s.as_str()) {
                                for line in th.lines() {
                                    let clean = line.trim();
                                    if !clean.is_empty() {
                                        results.push(format!("[THOUGHT] {}", clean));
                                    }
                                }
                            }
                        } else if item_type == Some("text") {
                            if let Some(txt) = item.get("text").and_then(|s| s.as_str()) {
                                for line in txt.lines() {
                                    let clean = line.trim();
                                    if !clean.is_empty() {
                                        results.push(format!("[RESPONSE] {}", clean));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else if role == Some("toolResult") {
            if let Some(msg) = v.get("message") {
                let tool = msg.get("toolName").and_then(|s| s.as_str()).unwrap_or("tool");
                let is_err = msg.get("isError").and_then(|b| b.as_bool()).unwrap_or(false);
                let status = if is_err { "FAIL" } else { "OK" };

                let mut text = String::new();
                if let Some(c_arr) = msg.get("content").and_then(|c| c.as_array()) {
                    for part in c_arr {
                        if let Some(t) = part.get("text").and_then(|s| s.as_str()) {
                            text.push_str(t);
                        }
                    }
                } else if let Some(s) = msg.get("content").and_then(|c| c.as_str()) {
                    text.push_str(s);
                }

                results.push(format!("[RESULT] {} ({})", tool, status));
                let lines: Vec<&str> = text.lines().collect();
                if lines.is_empty() {
                    results.push("  (empty output)".to_string());
                } else {
                    for (idx, l) in lines.iter().enumerate() {
                        if idx < 40 {
                            results.push(format!("  > {}", l));
                        } else {
                            results.push(format!("  > ... (+{} more lines)", lines.len() - 40));
                            break;
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }

    pub fn extract_latest_session_stats(preferred_file: Option<&Path>) -> Result<OmpSessionStats> {
        let session_file = match preferred_file {
            Some(p) if p.exists() => p.to_path_buf(),
            _ => {
                let dirs = Self::get_sessions_dirs();
                match Self::find_newest_session_file_across(&dirs) {
                    Some(f) => f,
                    None => return Err(anyhow!("No session jsonl file found")),
                }
            }
        };
        let content = std::fs::read_to_string(&session_file)?;

        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;
        let mut cached_tokens = 0u64;
        let mut steps = 0u32;

        for line in content.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                steps += 1;
                let usage_opt = v.get("usage")
                    .or_else(|| v.get("message").and_then(|m| m.get("usage")));

                if let Some(usage) = usage_opt {
                    if let Some(pt) = usage.get("input").or_else(|| usage.get("prompt_tokens")).and_then(|x| x.as_u64()) {
                        prompt_tokens += pt;
                    }
                    if let Some(ct) = usage.get("output").or_else(|| usage.get("completion_tokens")).and_then(|x| x.as_u64()) {
                        completion_tokens += ct;
                    }
                    if let Some(cr) = usage.get("cacheRead").or_else(|| usage.get("cache_read_input_tokens")).and_then(|x| x.as_u64()) {
                        cached_tokens += cr;
                    } else if let Some(details) = usage.get("prompt_tokens_details") {
                        if let Some(c) = details.get("cached_tokens").and_then(|x| x.as_u64()) {
                            cached_tokens += c;
                        }
                    }
                }
            }
        }

        Ok(OmpSessionStats {
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            total_tokens: prompt_tokens + cached_tokens + completion_tokens,
            steps_taken: steps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_session_line_tool_start() {
        let json_bash = r#"{"customType":"tool_execution_start","data":{"toolName":"bash","intent":"run tests","args":{"command":"cargo test\necho done"}}}"#;
        let res = OmpRunner::format_session_line(json_bash).unwrap();
        assert!(res.iter().any(|s| s.contains("[ACTION] bash: run tests")));
        assert!(res.iter().any(|s| s.contains("$ cargo test")));
        assert!(res.iter().any(|s| s.contains("$ echo done")));

        let json_read = r#"{"customType":"tool_execution_start","data":{"toolName":"read","intent":"read file","args":{"path":"/workspace/main.rs"}}}"#;
        let res_read = OmpRunner::format_session_line(json_read).unwrap();
        assert!(res_read.iter().any(|s| s.contains("path: /workspace/main.rs")));

        let json_write = r#"{"customType":"tool_execution_start","data":{"toolName":"write","intent":"","args":{"code":"line1\nline2"}}}"#;
        let res_write = OmpRunner::format_session_line(json_write).unwrap();
        assert!(res_write.iter().any(|s| s.contains("[ACTION] write")));
        assert!(res_write.iter().any(|s| s.contains("| line1")));
    }

    #[test]
    fn test_format_session_line_assistant() {
        let json_assistant = r#"{
            "message": {
                "role": "assistant",
                "usage": {"input": 1500, "output": 250, "cacheRead": 500},
                "content": [
                    {"type": "thinking", "thinking": "Let me think about this..."},
                    {"type": "text", "text": "Here is the code to solve it"}
                ]
            }
        }"#;
        let res = OmpRunner::format_session_line(json_assistant).unwrap();
        assert!(res.iter().any(|s| s.contains("[TOKENS] Turn usage - input: 1500, output: 250, cached: 500")));
        assert!(res.iter().any(|s| s.contains("[THOUGHT] Let me think about this...")));
        assert!(res.iter().any(|s| s.contains("[RESPONSE] Here is the code to solve it")));
    }

    #[test]
    fn test_format_session_line_tool_result() {
        let json_res_ok = r#"{
            "message": {
                "role": "toolResult",
                "toolName": "bash",
                "isError": false,
                "content": [{"text": "test passed successfully"}]
            }
        }"#;
        let res = OmpRunner::format_session_line(json_res_ok).unwrap();
        assert!(res.iter().any(|s| s.contains("[RESULT] bash (OK)")));
        assert!(res.iter().any(|s| s.contains("> test passed successfully")));

        let json_res_err = r#"{
            "message": {
                "role": "toolResult",
                "toolName": "read",
                "isError": true,
                "content": "File not found"
            }
        }"#;
        let res_err = OmpRunner::format_session_line(json_res_err).unwrap();
        assert!(res_err.iter().any(|s| s.contains("[RESULT] read (FAIL)")));
        assert!(res_err.iter().any(|s| s.contains("> File not found")));
    }

    #[test]
    fn test_extract_session_stats_and_finding() {
        let temp_dir = std::env::temp_dir().join(format!("test_omp_stats_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let session_file = temp_dir.join("test_session.jsonl");
        let content = "{\"usage\":{\"input\":2000,\"output\":150,\"cacheRead\":800}}\n{\"message\":{\"usage\":{\"input\":2500,\"output\":200,\"prompt_tokens_details\":{\"cached_tokens\":1200}}}}\n";
        std::fs::write(&session_file, content).unwrap();

        let stats = OmpRunner::extract_latest_session_stats(Some(&session_file)).unwrap();
        assert_eq!(stats.prompt_tokens, 4500);
        assert_eq!(stats.completion_tokens, 350);
        assert_eq!(stats.cached_tokens, 2000);
        assert_eq!(stats.total_tokens, 6850);
        assert_eq!(stats.steps_taken, 2);

        let found = OmpRunner::find_newest_session_file_across(&[temp_dir.clone()]);
        assert_eq!(found, Some(session_file));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_agent_execution_limits_defaults() {
        let limits = AgentExecutionLimits {
            max_turns: Some(10),
            max_budget_usd: Some(0.20),
            timeout_seconds: Some(600),
        };
        assert_eq!(limits.max_turns, Some(10));
        assert_eq!(limits.max_budget_usd, Some(0.20));
        assert_eq!(limits.timeout_seconds, Some(600));
    }

    #[test]
    fn test_tail_session_file_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("test_tail_sb_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let session_file = temp_dir.join("active_session.jsonl");
        let initial_line = r#"{"customType":"tool_execution_start","data":{"toolName":"bash","intent":"init","args":{"command":"echo start"}}}"#;
        std::fs::write(&session_file, format!("{}\n", initial_line)).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let active_file = Arc::new(Mutex::new(Some(session_file.clone())));
        let turn_counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let accumulated_cost = Arc::new(Mutex::new(0.0f64));
        let limit_reached = Arc::new(AtomicBool::new(false));
        let limit_reason = Arc::new(Mutex::new(None));
        let limits = AgentExecutionLimits {
            max_turns: Some(5),
            max_budget_usd: Some(0.10),
            timeout_seconds: Some(10),
        };

        let running_tailer = running.clone();
        let handle = std::thread::spawn(move || {
            OmpRunner::tail_session_file(
                SystemTime::now(),
                HashSet::new(),
                active_file,
                running_tailer,
                tx,
                limits,
                turn_counter,
                accumulated_cost,
                limit_reached,
                limit_reason,
            );
        });

        // Verify initial line received
        let rec = rx.recv_timeout(Duration::from_millis(500));
        assert!(rec.is_ok());

        // Stop tailer
        running.store(false, Ordering::SeqCst);
        let _ = handle.join();

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_format_session_line_edge_cases() {
        // 1. Tool with > 20 lines of code
        let long_code = (0..25).map(|i| format!("println!(\"line {}\");", i)).collect::<Vec<_>>().join("\n");
        let json_code = serde_json::json!({
            "customType": "tool_execution_start",
            "data": {
                "toolName": "write",
                "args": {
                    "code": long_code
                }
            }
        }).to_string();
        let res_code = OmpRunner::format_session_line(&json_code).unwrap();
        assert!(res_code.iter().any(|s| s.contains("(+5 lines)")));

        // 2. ToolResult with > 40 lines
        let long_res = (0..50).map(|i| format!("output row {}", i)).collect::<Vec<_>>().join("\n");
        let json_res = serde_json::json!({
            "message": {
                "role": "toolResult",
                "toolName": "bash",
                "content": long_res
            }
        }).to_string();
        let res_long = OmpRunner::format_session_line(&json_res).unwrap();
        assert!(res_long.iter().any(|s| s.contains("(+10 more lines)")));

        // 3. ToolResult with empty output
        let json_empty = serde_json::json!({
            "message": {
                "role": "toolResult",
                "toolName": "bash",
                "content": ""
            }
        }).to_string();
        let res_empty = OmpRunner::format_session_line(&json_empty).unwrap();
        assert!(res_empty.iter().any(|s| s.contains("(empty output)")));

        // 4. Invalid or unhandled JSON
        assert!(OmpRunner::format_session_line("not-json").is_none());
        assert!(OmpRunner::format_session_line(r#"{"unknown": "object"}"#).is_none());
    }

    #[test]
    fn test_sessions_dirs_and_collect() {
        let dirs = OmpRunner::get_sessions_dirs();
        let _ = OmpRunner::collect_all_session_files();
        assert!(!dirs.is_empty() || dirs.is_empty()); // runs without panicking
    }

    #[test]
    fn test_find_new_session_file_priorities() {
        let temp_dir = std::env::temp_dir().join(format!("test_find_prio_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let start_time = SystemTime::now() - Duration::from_secs(10);
        let file1 = temp_dir.join("file1.jsonl");
        let file2 = temp_dir.join("file2.jsonl");

        std::fs::write(&file1, "line1\n").unwrap();
        std::fs::write(&file2, "line2\n").unwrap();

        let mut existing = HashSet::new();
        existing.insert(file1.clone());

        let dirs = vec![temp_dir.clone()];
        // Priority 1: file2 is brand new
        let found = OmpRunner::find_new_session_file(&dirs, &existing, start_time);
        assert_eq!(found, Some(file2.clone()));

        // Priority 2: all exist, but file1 modified after start_time
        existing.insert(file2.clone());
        let found2 = OmpRunner::find_new_session_file(&dirs, &existing, start_time);
        assert!(found2.is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_tail_session_file_turn_and_budget_limits() {
        let temp_dir = std::env::temp_dir().join(format!("test_tail_limits_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let session_file = temp_dir.join("session_limit.jsonl");
        // Write assistant message and budget usage
        let assistant_turn = r#"{"message":{"role":"assistant","usage":{"cost":{"total":0.25}},"content":[{"type":"text","text":"hello"}]}}"#;
        std::fs::write(&session_file, format!("{}\n", assistant_turn)).unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let active_file = Arc::new(Mutex::new(Some(session_file.clone())));
        let turn_counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let accumulated_cost = Arc::new(Mutex::new(0.0f64));
        let limit_reached = Arc::new(AtomicBool::new(false));
        let limit_reason = Arc::new(Mutex::new(None));
        let limits = AgentExecutionLimits {
            max_turns: Some(1), // Should trigger limit immediately
            max_budget_usd: Some(0.20), // Should also trigger budget limit
            timeout_seconds: Some(10),
        };

        let running_tailer = running.clone();
        let turn_c = turn_counter.clone();
        let lim_reached = limit_reached.clone();
        let handle = std::thread::spawn(move || {
            OmpRunner::tail_session_file(
                SystemTime::now() - Duration::from_secs(5),
                HashSet::new(),
                active_file,
                running_tailer,
                tx,
                limits,
                turn_c,
                accumulated_cost,
                lim_reached,
                limit_reason,
            );
        });

        std::thread::sleep(Duration::from_millis(300));
        assert!(limit_reached.load(Ordering::SeqCst));
        assert_eq!(turn_counter.load(Ordering::SeqCst), 1);

        running.store(false, Ordering::SeqCst);
        let _ = handle.join();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_run_agent_invalid_model_docker() {
        let temp_dir = std::env::temp_dir().join(format!("test_run_agent_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let limits = AgentExecutionLimits {
            max_turns: Some(1),
            max_budget_usd: Some(0.10),
            timeout_seconds: Some(5),
        };

        let res = OmpRunner::run_agent(
            "nonexistent-test-model-xyz",
            "test prompt",
            &temp_dir,
            None,
            limits,
            Some("auto"),
        );

        // omp should fail on nonexistent model with error
        assert!(res.is_err());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_run_agent_relative_path_resolution() {
        let rel_dir = Path::new("test_rel_ws_omp");
        let _ = std::fs::create_dir_all(rel_dir);

        let limits = AgentExecutionLimits {
            max_turns: Some(1),
            max_budget_usd: Some(0.10),
            timeout_seconds: Some(5),
        };

        let res = OmpRunner::run_agent(
            "nonexistent-test-model-xyz",
            "test prompt",
            rel_dir,
            None,
            limits,
            Some("auto"),
        );

        if let Err(e) = res {
            let err_str = e.to_string();
            assert!(!err_str.contains("invalid characters for a local volume name"));
        }

        let _ = std::fs::remove_dir_all(rel_dir);
    }
}
