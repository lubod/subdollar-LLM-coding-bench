use anyhow::Result;
use std::path::Path;
use std::process::Command;
use tracing::info;

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

    pub fn start_candidate_in_docker(&self, workdir: &Path, port: u16) -> Result<()> {
        info!("Starting candidate clone in Docker on port {}", port);
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-candidate"])
            .output();

        let canonical_workdir = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
        let mount_arg = format!("{}:/workspace", canonical_workdir.display());
        let port_arg = format!("{}:{}", port, port);

        let _ = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
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

    pub fn cleanup(&self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", "subdollar-ref-redis", "subdollar-ref-http", "subdollar-candidate"])
            .output();
    }
}
