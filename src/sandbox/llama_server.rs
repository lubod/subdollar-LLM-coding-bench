use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Duration;
use tracing::{info, warn};

pub const LLAMA_CONTAINER_NAME: &str = "subdollar-llama";
pub const LLAMA_IMAGE: &str = "ghcr.io/ggml-org/llama.cpp:server";
pub const LLAMA_PORT: u16 = 8000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaServerStatus {
    pub running: bool,
    pub model: Option<String>,
    pub status_text: String,
    pub endpoint: String,
    pub logs: String,
}

pub struct LlamaServerManager;

impl LlamaServerManager {
    pub fn resolve_preset(preset: &str) -> String {
        let p = preset.trim();
        match p {
            "qwen2.5-coder-1.5b" | "qwen-1.5b" | "qwen1.5b" => {
                "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF:Q4_K_M".to_string()
            }
            "qwen2.5-coder-7b" | "qwen-7b" | "qwen7b" => {
                "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF:Q4_K_M".to_string()
            }
            "llama-3.2-3b" | "llama3.2-3b" => {
                "bartowski/Llama-3.2-3B-Instruct-GGUF:Q4_K_M".to_string()
            }
            custom => custom.to_string(),
        }
    }

    pub async fn status() -> LlamaServerStatus {
        let endpoint = format!("http://127.0.0.1:{}/v1", LLAMA_PORT);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .unwrap_or_default();

        let health_url = format!("http://127.0.0.1:{}/health", LLAMA_PORT);
        let models_url = format!("http://127.0.0.1:{}/v1/models", LLAMA_PORT);

        let is_healthy = match client.get(&health_url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };

        if is_healthy {
            let loaded_model = if let Ok(resp) = client.get(&models_url).send().await {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    json.get("data")
                        .and_then(|d| d.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|m| m.get("id"))
                        .and_then(|id| id.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            } else {
                None
            };

            let logs = Self::get_recent_logs(8).unwrap_or_default();
            return LlamaServerStatus {
                running: true,
                model: loaded_model.clone(),
                status_text: format!(
                    "Running · {}",
                    loaded_model.unwrap_or_else(|| "Ready".to_string())
                ),
                endpoint,
                logs,
            };
        }

        // Check if container exists in docker
        let inspect_out = Command::new("docker")
            .args([
                "inspect",
                "--format={{.State.Status}}",
                LLAMA_CONTAINER_NAME,
            ])
            .output();

        let (running, status_text) = match inspect_out {
            Ok(out) if out.status.success() => {
                let st = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if st == "running" {
                    (
                        false,
                        "Downloading / Loading GGUF model into memory...".to_string(),
                    )
                } else {
                    (false, format!("Container {}", st))
                }
            }
            _ => (false, "Stopped".to_string()),
        };

        let logs = Self::get_recent_logs(8).unwrap_or_default();
        LlamaServerStatus {
            running,
            model: None,
            status_text,
            endpoint,
            logs,
        }
    }

    pub fn start(model_preset: &str) -> Result<()> {
        let repo = Self::resolve_preset(model_preset);
        info!("Starting llama-server container with model repo: {}", repo);

        let _ = Command::new("docker")
            .args(["rm", "-f", LLAMA_CONTAINER_NAME])
            .output();

        let port_mapping = format!("{}:8080", LLAMA_PORT);
        let out = Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                LLAMA_CONTAINER_NAME,
                "--restart",
                "unless-stopped",
                "-p",
                &port_mapping,
                "-v",
                "subdollar-models:/root/.cache",
                LLAMA_IMAGE,
                "--hf-repo",
                &repo,
                "--host",
                "0.0.0.0",
                "--port",
                "8080",
                "-c",
                "16384",
                "-t",
                "16",
                "--jinja",
            ])
            .output()
            .map_err(|e| anyhow!("Failed to execute docker run: {}", e))?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(anyhow!("docker run failed: {}", err));
        }

        Ok(())
    }

    pub fn stop() -> Result<()> {
        info!("Stopping and removing {} container", LLAMA_CONTAINER_NAME);
        let out = Command::new("docker")
            .args(["rm", "-f", LLAMA_CONTAINER_NAME])
            .output()
            .map_err(|e| anyhow!("Failed to remove container: {}", e))?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            warn!("docker rm returned error: {}", err);
        }
        Ok(())
    }

    pub fn get_recent_logs(lines: usize) -> Result<String> {
        let out = Command::new("docker")
            .args(["logs", "--tail", &lines.to_string(), LLAMA_CONTAINER_NAME])
            .output()?;

        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            let err_text = String::from_utf8_lossy(&out.stderr).to_string();
            if text.is_empty() {
                Ok(err_text)
            } else {
                Ok(text)
            }
        } else {
            Ok(String::new())
        }
    }

    pub async fn ensure_running_for_model(model: &str) -> Result<()> {
        let model_lower = model.to_lowercase();
        let target_preset = if model_lower.contains("7b") {
            "qwen2.5-coder-7b"
        } else if model_lower.contains("3.2") || model_lower.contains("3b") {
            "llama-3.2-3b"
        } else {
            "qwen2.5-coder-1.5b"
        };
        let target_repo = Self::resolve_preset(target_preset).to_lowercase();

        let st = Self::status().await;
        if st.running {
            if let Some(ref current_model) = st.model {
                let curr = current_model.to_lowercase();
                let is_matching = curr == target_repo
                    || (target_preset == "qwen2.5-coder-7b" && (curr.contains("7b") || curr.contains("coder-7b")))
                    || (target_preset == "qwen2.5-coder-1.5b" && (curr.contains("1.5b") || curr.contains("coder-1.5b")))
                    || (target_preset == "llama-3.2-3b" && curr.contains("3.2"));
                if is_matching {
                    return Ok(());
                }
            }
            info!(
                "Running llama-server model ({:?}) differs from requested '{}'. Restarting...",
                st.model, target_preset
            );
        }

        Self::start(target_preset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_preset() {
        assert_eq!(
            LlamaServerManager::resolve_preset("qwen2.5-coder-1.5b"),
            "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF:Q4_K_M"
        );
        assert_eq!(
            LlamaServerManager::resolve_preset("qwen2.5-coder-7b"),
            "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF:Q4_K_M"
        );
        assert_eq!(
            LlamaServerManager::resolve_preset("my-user/my-repo:Q5_K_M"),
            "my-user/my-repo:Q5_K_M"
        );
    }

    #[tokio::test]
    async fn test_llama_server_status_structure() {
        let st = LlamaServerManager::status().await;
        assert_eq!(st.endpoint, "http://127.0.0.1:8000/v1");
    }
}
