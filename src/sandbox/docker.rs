use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

pub struct SandboxManager;

impl SandboxManager {
    pub fn new() -> Self {
        Self
    }

    pub fn start_reference_redis(&self, port: u16) -> Result<()> {
        info!("Starting ground-truth reference Redis in Docker on port {}", port);
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-ref-redis"])
            .output();

        let _ = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                "subdollar-ref-redis",
                "-p",
                &format!("{}:6379", port),
                "redis:alpine",
            ])
            .output();

        Ok(())
    }

    pub fn start_reference_http(&self, port: u16) -> Result<()> {
        info!("Starting reference Nginx in Docker on port {}", port);
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-ref-http"])
            .output();

        let _ = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                "subdollar-ref-http",
                "--memory=512m",
                "--cpus=1.0",
                "-p",
                &format!("{}:80", port),
                "nginx:alpine",
            ])
            .output();

        Ok(())
    }

    pub fn start_reference_dns(&self, port: u16) -> Result<()> {
        info!("Starting reference DNS server in Docker on UDP port {}", port);
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-ref-dns"])
            .output();

        let _ = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                "subdollar-ref-dns",
                "--memory=512m",
                "--cpus=1.0",
                "-p",
                &format!("{}:53/udp", port),
                "coredns/coredns",
            ])
            .output();

        Ok(())
    }

    /// Recursively find a file by name within directory
    fn find_file_recursive(dir: &Path, filename: &str) -> Option<PathBuf> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(filename) {
                    return Some(path);
                } else if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with('.') || name == "target" || name == "node_modules" {
                            continue;
                        }
                    }
                    if let Some(found) = Self::find_file_recursive(&path, filename) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// Recursively check if directory contains a runnable candidate.
    /// If found in a subdirectory, creates a forwarding root start.sh.
    pub fn ensure_runnable_candidate(&self, workdir: &Path) -> Result<bool> {
        if !workdir.exists() {
            return Ok(false);
        }

        let root_dockerfile = workdir.join("Dockerfile");
        if root_dockerfile.exists() {
            return Ok(true);
        }

        let root_start_sh = workdir.join("start.sh");
        if root_start_sh.exists() {
            return Ok(true);
        }

        // Search for Dockerfile in subdirectories
        if let Some(found_df) = Self::find_file_recursive(workdir, "Dockerfile") {
            if found_df != root_dockerfile {
                info!("Found nested Dockerfile at {:?}. Hoisting to root workspace...", found_df);
                let _ = fs::copy(&found_df, &root_dockerfile);
                return Ok(true);
            }
        }

        // Search for start.sh in subdirectories
        if let Some(found_sh) = Self::find_file_recursive(workdir, "start.sh") {
            if found_sh != root_start_sh {
                info!("Found nested start.sh at {:?}. Creating root bridge start.sh...", found_sh);
                if let Ok(rel_path) = found_sh.strip_prefix(workdir) {
                    let parent_dir = rel_path.parent().unwrap_or(Path::new(""));
                    let script_name = rel_path.file_name().unwrap_or_default().to_string_lossy();
                    let bridge_content = format!(
                        "#!/bin/bash\ncd /workspace/{}\nchmod +x ./{}\nexec ./{}\n",
                        parent_dir.display(),
                        script_name,
                        script_name
                    );
                    let _ = fs::write(&root_start_sh, bridge_content);
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Start the candidate server container with resource limits
    pub fn start_candidate_in_docker(&self, workdir: &Path, port: u16) -> Result<()> {
        info!("Starting candidate server in isolated Docker container for port {}", port);

        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-candidate"])
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
        let port_tcp = format!("{}:{}/tcp", port, port);
        let port_udp = format!("{}:{}/udp", port, port);

        // Ensure runnable (detects nested start.sh / Dockerfile)
        let _ = self.ensure_runnable_candidate(&canonical_workdir);

        // CASE A: Dockerfile exists -> Build and run custom Docker container
        let dockerfile_path = canonical_workdir.join("Dockerfile");
        if dockerfile_path.exists() {
            info!("Detected Dockerfile in candidate workspace! Building candidate Docker image...");
            let build_status = Command::new("docker")
                .args(["build", "-t", "subdollar-candidate-custom", "."])
                .current_dir(&canonical_workdir)
                .output();

            match build_status {
                Ok(out) if out.status.success() => {
                    info!("Successfully built custom Docker image 'subdollar-candidate-custom'");
                    let run_res = Command::new("docker")
                        .args([
                            "run",
                            "-d",
                            "--name",
                            "subdollar-candidate",
                            "--memory=2g",
                            "--cpus=2.0",
                            "--pids-limit=256",
                            "-p",
                            &port_tcp,
                            "-p",
                            &port_udp,
                            "subdollar-candidate-custom",
                        ])
                        .output();
                    if let Ok(run_out) = run_res {
                        if !run_out.status.success() {
                            warn!("Failed to start custom container: {}", String::from_utf8_lossy(&run_out.stderr));
                        }
                    }
                    return Ok(());
                }
                Ok(out) => {
                    warn!(
                        "Candidate Dockerfile build failed: {}. Falling back to start.sh...",
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => {
                    warn!("Failed to execute docker build: {}. Falling back to start.sh...", e);
                }
            }
        }

        // CASE B: Standard Sandbox with start.sh
        let mount_arg = format!("{}:/workspace", canonical_workdir.display());
        let _ = Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                "subdollar-candidate",
                "--memory=2g",
                "--cpus=2.0",
                "--pids-limit=256",
                "-v",
                &mount_arg,
                "-p",
                &port_tcp,
                "-p",
                &port_udp,
                "subdollar-sandbox",
                "/bin/bash",
                "-c",
                "cd /workspace && chmod +x start.sh && ./start.sh",
            ])
            .output();

        Ok(())
    }

    /// Actively poll the target TCP port until the candidate is ready, up to timeout_secs
    pub async fn wait_for_port(&self, port: u16, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        while start.elapsed() < timeout {
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                // Give a 500ms stabilization window
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        false
    }

    pub fn get_candidate_logs(&self) -> String {
        let output = Command::new("docker")
            .args(["logs", "--tail", "100", "subdollar-candidate"])
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let mut logs = String::new();
                if !stdout.is_empty() {
                    logs.push_str(&format!("--- Container stdout ---\n{}\n", stdout));
                }
                if !stderr.is_empty() {
                    logs.push_str(&format!("--- Container stderr ---\n{}\n", stderr));
                }
                if logs.is_empty() {
                    logs.push_str("(No output logged by container)");
                }
                logs
            }
            Err(e) => format!("Failed to read container logs: {}", e),
        }
    }

    pub fn cleanup(&self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-ref-redis", "subdollar-ref-http", "subdollar-candidate", "subdollar-omp-agent"])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn test_ensure_runnable_candidate_scenarios() {
        let temp_dir = std::env::temp_dir().join(format!("test_docker_sb_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let sm = SandboxManager::new();

        // 1. Empty dir
        assert_eq!(sm.ensure_runnable_candidate(&temp_dir).unwrap(), false);

        // 2. Root Dockerfile
        fs::write(temp_dir.join("Dockerfile"), "FROM alpine").unwrap();
        assert_eq!(sm.ensure_runnable_candidate(&temp_dir).unwrap(), true);
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 3. Nested Dockerfile
        let nested = temp_dir.join("subdir/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("Dockerfile"), "FROM alpine").unwrap();
        assert_eq!(sm.ensure_runnable_candidate(&temp_dir).unwrap(), true);
        assert!(temp_dir.join("Dockerfile").exists());
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));
        let _ = fs::remove_dir_all(&nested);

        // 4. Root start.sh
        fs::write(temp_dir.join("start.sh"), "#!/bin/bash\necho ok").unwrap();
        assert_eq!(sm.ensure_runnable_candidate(&temp_dir).unwrap(), true);
        let _ = fs::remove_file(temp_dir.join("start.sh"));

        // 5. Nested start.sh
        let nested_sh = temp_dir.join("project");
        fs::create_dir_all(&nested_sh).unwrap();
        fs::write(nested_sh.join("start.sh"), "#!/bin/bash\necho nested").unwrap();
        assert_eq!(sm.ensure_runnable_candidate(&temp_dir).unwrap(), true);
        assert!(temp_dir.join("start.sh").exists());
        let root_content = fs::read_to_string(temp_dir.join("start.sh")).unwrap();
        assert!(root_content.contains("/workspace/project"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_wait_for_port() {
        let sm = SandboxManager::new();

        // 1. Unbound port should return false
        let ok = sm.wait_for_port(59995, 1).await;
        assert!(!ok);

        // 2. Active port should return true
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let ok2 = sm.wait_for_port(port, 2).await;
        assert!(ok2);
    }

    #[test]
    fn test_docker_cleanup_and_logs() {
        let sm = SandboxManager::new();
        sm.cleanup();
        let logs = sm.get_candidate_logs();
        assert!(!logs.is_empty());
    }

    #[test]
    fn test_start_and_cleanup_reference_containers() {
        let sm = SandboxManager::new();
        let res_redis = sm.start_reference_redis(59994);
        assert!(res_redis.is_ok());
        let res_http = sm.start_reference_http(59993);
        assert!(res_http.is_ok());
        sm.cleanup();
    }

    #[test]
    fn test_start_candidate_in_docker_start_sh_and_dockerfile() {
        let temp_dir = std::env::temp_dir().join(format!("test_docker_run_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let sm = SandboxManager::new();

        // 1. start.sh case
        fs::write(temp_dir.join("start.sh"), "#!/bin/bash\necho candidate running\n").unwrap();
        let res_sh = sm.start_candidate_in_docker(&temp_dir, 59990);
        assert!(res_sh.is_ok());
        sm.cleanup();

        // 2. Dockerfile case
        let _ = fs::remove_file(temp_dir.join("start.sh"));
        fs::write(temp_dir.join("Dockerfile"), "FROM alpine\nCMD [\"echo\", \"done\"]\n").unwrap();
        let res_df = sm.start_candidate_in_docker(&temp_dir, 59989);
        assert!(res_df.is_ok());
        sm.cleanup();

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
