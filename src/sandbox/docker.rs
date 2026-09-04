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
                "-p",
                &format!("{}:80", port),
                "nginx:alpine",
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

    /// Check if candidate has either a Dockerfile or start.sh (root or nested)
    pub fn ensure_runnable_candidate(&self, workdir: &Path) -> Result<bool> {
        let canonical_workdir = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());

        // 1. Check if root Dockerfile exists
        if canonical_workdir.join("Dockerfile").exists() {
            return Ok(true);
        }

        // 2. Check if nested Dockerfile exists
        if let Some(nested_dockerfile) = Self::find_file_recursive(&canonical_workdir, "Dockerfile") {
            info!("Found nested Dockerfile at: {}", nested_dockerfile.display());
            let root_df = canonical_workdir.join("Dockerfile");
            if !root_df.exists() {
                let _ = fs::copy(&nested_dockerfile, &root_df);
            }
            return Ok(true);
        }

        // 3. Check if root start.sh exists
        if canonical_workdir.join("start.sh").exists() {
            return Ok(true);
        }

        // 4. Search for nested start.sh and generate wrapper
        if let Some(nested_start) = Self::find_file_recursive(&canonical_workdir, "start.sh") {
            info!("Found nested start.sh at: {}", nested_start.display());
            if let Ok(rel) = nested_start.strip_prefix(&canonical_workdir) {
                if let Some(parent) = rel.parent() {
                    let parent_str = parent.display().to_string();
                    if !parent_str.is_empty() {
                        let wrapper_content = format!(
                            "#!/bin/bash\ncd \"/workspace/{}\"\nchmod +x start.sh 2>/dev/null || true\nexec ./start.sh\n",
                            parent_str
                        );
                        let root_start = canonical_workdir.join("start.sh");
                        let _ = fs::write(&root_start, wrapper_content);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = fs::set_permissions(&root_start, fs::Permissions::from_mode(0o755));
                        }
                        info!("Created root start.sh forwarding to subfolder: {}", parent_str);
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    pub fn start_candidate_in_docker(&self, workdir: &Path, port: u16) -> Result<()> {
        info!("Starting candidate clone in Docker on port {}", port);
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-candidate"])
            .output();

        let canonical_workdir = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
        let port_arg = format!("{}:{}", port, port);

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
                            "-p",
                            &port_arg,
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
                "-v",
                &mount_arg,
                "-p",
                &port_arg,
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
