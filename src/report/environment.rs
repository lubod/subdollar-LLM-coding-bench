use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub total_memory: String,
    pub docker_version: String,
    pub rust_version: String,
    pub python_version: String,
    pub node_version: String,
    pub git_commit: String,
    pub git_branch: String,
    pub timestamp: String,
}

impl EnvironmentInfo {
    pub fn detect() -> Self {
        let os = Self::detect_os();
        let kernel = Self::cmd_output("uname", &["-s", "-r"]);
        let arch = Self::cmd_output("uname", &["-m"]);
        let (cpu_model, cpu_cores) = Self::detect_cpu();
        let total_memory = Self::detect_memory();
        let docker_version = Self::cmd_output("docker", &["--version"]);
        let rust_version = Self::detect_rust();
        let python_version = Self::cmd_output("python3", &["--version"]);
        let node_version = Self::cmd_output("node", &["--version"]);
        let git_commit = Self::detect_git_commit();
        let git_branch = Self::detect_git_branch();
        let timestamp = Utc::now().to_rfc3339();

        Self {
            os,
            kernel,
            arch,
            cpu_model,
            cpu_cores,
            total_memory,
            docker_version,
            rust_version,
            python_version,
            node_version,
            git_commit,
            git_branch,
            timestamp,
        }
    }

    fn detect_os() -> String {
        if let Ok(content) = fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(stripped) = line.strip_prefix("PRETTY_NAME=") {
                    return stripped.trim_matches('"').to_string();
                }
            }
        }
        std::env::consts::OS.to_string()
    }

    fn detect_cpu() -> (String, u32) {
        let mut model = "Unknown CPU".to_string();
        if let Ok(content) = fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(val) = line.split(':').nth(1) {
                        model = val.trim().to_string();
                        break;
                    }
                }
            }
        }
        let cores = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);

        (model, cores)
    }

    fn detect_memory() -> String {
        if let Ok(content) = fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(val) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = val.parse::<u64>() {
                            let gb = (kb as f64) / (1024.0 * 1024.0);
                            return format!("{:.1} GiB", gb);
                        }
                    }
                }
            }
        }
        "Unknown".to_string()
    }

    fn detect_rust() -> String {
        let direct = Self::cmd_output("rustc", &["--version"]);
        if direct != "N/A" && !direct.is_empty() {
            return direct;
        }
        if let Ok(output) = Command::new("su").args(["-", "ubuntu", "-c", "rustc --version"]).output() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        "N/A".to_string()
    }

    fn detect_git_commit() -> String {
        let out = Self::cmd_output("git", &["rev-parse", "--short", "HEAD"]);
        if out != "N/A" && !out.is_empty() {
            return out;
        }
        if let Ok(output) = Command::new("su").args(["-", "ubuntu", "-c", "cd /home/ubuntu/subdollar-LLM-coding-bench && git rev-parse --short HEAD"]).output() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        "unknown".to_string()
    }

    fn detect_git_branch() -> String {
        let out = Self::cmd_output("git", &["rev-parse", "--abbrev-ref", "HEAD"]);
        if out != "N/A" && !out.is_empty() {
            return out;
        }
        if let Ok(output) = Command::new("su").args(["-", "ubuntu", "-c", "cd /home/ubuntu/subdollar-LLM-coding-bench && git rev-parse --abbrev-ref HEAD"]).output() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        "master".to_string()
    }

    fn cmd_output(cmd: &str, args: &[&str]) -> String {
        match Command::new(cmd).args(args).output() {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            _ => "N/A".to_string(),
        }
    }

    pub fn to_markdown_table(&self) -> String {
        format!(
            "| Component | Specification |\n\
             |:---|:---|\n\
             | **Operating System** | {} |\n\
             | **Kernel & Architecture** | {} ({}) |\n\
             | **CPU Model** | {} ({} vCPUs) |\n\
             | **System Memory** | {} |\n\
             | **Docker Runtime** | {} |\n\
             | **Rust Version** | {} |\n\
             | **Python Version** | {} |\n\
             | **Node.js Version** | {} |\n\
             | **Git Baseline** | Commit `{}` (branch `{}`) |\n\
             | **Benchmark Captured** | {} |",
            self.os,
            self.kernel,
            self.arch,
            self.cpu_model,
            self.cpu_cores,
            self.total_memory,
            self.docker_version,
            self.rust_version,
            self.python_version,
            self.node_version,
            self.git_commit,
            self.git_branch,
            self.timestamp
        )
    }
}
