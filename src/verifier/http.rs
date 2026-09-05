use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::StageResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestSummary {
    pub stages: Vec<StageResult>,
    pub passed_count: u32,
    pub total_stages: u32,
    pub pass_rate: f64,
}

pub struct HttpVerifier {
    pub target_port: u16,
    pub client: reqwest::Client,
}

impl HttpVerifier {
    pub fn new(target_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        Self { target_port, client }
    }

    pub async fn run_all(&self) -> HttpTestSummary {
        let mut stages = Vec::new();
        stages.push(self.test_stage1_root().await);
        stages.push(self.test_stage2_not_found().await);
        stages.push(self.test_stage3_echo().await);
        stages.push(self.test_stage4_user_agent().await);
        stages.push(self.test_stage5_file_storage().await);

        let passed_count = stages.iter().filter(|s| s.passed).count() as u32;
        let total_stages = stages.len() as u32;
        let pass_rate = (passed_count as f64 / total_stages as f64) * 100.0;

        HttpTestSummary {
            stages,
            passed_count,
            total_stages,
            pass_rate,
        }
    }

    pub async fn test_stage1_root(&self) -> StageResult {
        let name = "Stage 1: GET / (Root 200 OK)".to_string();
        let url = format!("http://127.0.0.1:{}/", self.target_port);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if resp.status() == reqwest::StatusCode::OK {
                    StageResult { stage: 1, name, passed: true, error: None }
                } else {
                    StageResult { stage: 1, name, passed: false, error: Some(format!("Expected 200 OK, got {}", resp.status())) }
                }
            }
            Err(e) => StageResult { stage: 1, name, passed: false, error: Some(e.to_string()) },
        }
    }

    pub async fn test_stage2_not_found(&self) -> StageResult {
        let name = "Stage 2: 404 Not Found Handling".to_string();
        let url = format!("http://127.0.0.1:{}/nonexistent-route-1234", self.target_port);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    StageResult { stage: 2, name, passed: true, error: None }
                } else {
                    StageResult { stage: 2, name, passed: false, error: Some(format!("Expected 404 Not Found, got {}", resp.status())) }
                }
            }
            Err(e) => StageResult { stage: 2, name, passed: false, error: Some(e.to_string()) },
        }
    }

    pub async fn test_stage3_echo(&self) -> StageResult {
        let name = "Stage 3: GET /echo/{str}".to_string();
        let word = "subdollar_speed_test";
        let url = format!("http://127.0.0.1:{}/echo/{}", self.target_port, word);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::OK {
                    return StageResult { stage: 3, name, passed: false, error: Some(format!("Expected 200 OK, got {}", resp.status())) };
                }
                match resp.text().await {
                    Ok(text) if text.trim() == word => StageResult { stage: 3, name, passed: true, error: None },
                    Ok(text) => StageResult { stage: 3, name, passed: false, error: Some(format!("Body didn't match '{}', got: '{}'", word, text)) },
                    Err(e) => StageResult { stage: 3, name, passed: false, error: Some(e.to_string()) },
                }
            }
            Err(e) => StageResult { stage: 3, name, passed: false, error: Some(e.to_string()) },
        }
    }

    pub async fn test_stage4_user_agent(&self) -> StageResult {
        let name = "Stage 4: GET /user-agent Header Echo".to_string();
        let ua = "SubDollarBench-Agent/1.0";
        let url = format!("http://127.0.0.1:{}/user-agent", self.target_port);
        match self.client.get(&url).header("User-Agent", ua).send().await {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::OK {
                    return StageResult { stage: 4, name, passed: false, error: Some(format!("Expected 200 OK, got {}", resp.status())) };
                }
                match resp.text().await {
                    Ok(text) if text.trim() == ua => StageResult { stage: 4, name, passed: true, error: None },
                    Ok(text) => StageResult { stage: 4, name, passed: false, error: Some(format!("Expected UA '{}', got '{}'", ua, text)) },
                    Err(e) => StageResult { stage: 4, name, passed: false, error: Some(e.to_string()) },
                }
            }
            Err(e) => StageResult { stage: 4, name, passed: false, error: Some(e.to_string()) },
        }
    }

    pub async fn test_stage5_file_storage(&self) -> StageResult {
        let name = "Stage 5: POST & GET /files/{filename}".to_string();
        let filename = "bench_artifact.txt";
        let content = "Hello from SubDollarBench automated test payload!";
        let post_url = format!("http://127.0.0.1:{}/files/{}", self.target_port, filename);
        let get_url = format!("http://127.0.0.1:{}/files/{}", self.target_port, filename);

        match self.client.post(&post_url).body(content).send().await {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::CREATED && !resp.status().is_success() {
                    return StageResult { stage: 5, name, passed: false, error: Some(format!("POST /files failed with status: {}", resp.status())) };
                }
            }
            Err(e) => return StageResult { stage: 5, name, passed: false, error: Some(format!("POST error: {}", e)) },
        }

        match self.client.get(&get_url).send().await {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::OK {
                    return StageResult { stage: 5, name, passed: false, error: Some(format!("GET /files failed with status: {}", resp.status())) };
                }
                match resp.text().await {
                    Ok(text) if text.trim() == content => StageResult { stage: 5, name, passed: true, error: None },
                    Ok(text) => StageResult { stage: 5, name, passed: false, error: Some(format!("File content mismatch! Expected '{}', got '{}'", content, text)) },
                    Err(e) => StageResult { stage: 5, name, passed: false, error: Some(e.to_string()) },
                }
            }
            Err(e) => StageResult { stage: 5, name, passed: false, error: Some(e.to_string()) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_http_verifier_all_stages_pass() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let file_store: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));

        let store_clone = file_store.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let store = store_clone.clone();

                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = socket.read(&mut buf).await {
                        if n == 0 { break; }
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let first_line = req.lines().next().unwrap_or("");
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        if parts.len() < 2 {
                            break;
                        }
                        let method = parts[0];
                        let path = parts[1];

                        let response = if method == "GET" && path == "/" {
                            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nOK".to_string()
                        } else if method == "GET" && path.starts_with("/echo/") {
                            let echo_str = &path["/echo/".len()..];
                            format!(
                                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                                echo_str.len(),
                                echo_str
                            )
                        } else if method == "GET" && path == "/user-agent" {
                            let mut ua = "unknown";
                            for line in req.lines() {
                                if line.to_lowercase().starts_with("user-agent:") {
                                    ua = line["user-agent:".len()..].trim();
                                }
                            }
                            format!(
                                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                                ua.len(),
                                ua
                            )
                        } else if method == "POST" && path.starts_with("/files/") {
                            let filename = path["/files/".len()..].to_string();
                            let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
                            store.lock().unwrap().insert(filename, body.to_string());
                            "HTTP/1.1 201 Created\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_string()
                        } else if method == "GET" && path.starts_with("/files/") {
                            let filename = &path["/files/".len()..];
                            if let Some(content) = store.lock().unwrap().get(filename) {
                                format!(
                                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                                    content.len(),
                                    content
                                )
                            } else {
                                "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_string()
                            }
                        } else {
                            "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_string()
                        };

                        let _ = socket.write_all(response.as_bytes()).await;
                    }
                });
            }
        });

        let verifier = HttpVerifier::new(port);
        let summary = verifier.run_all().await;
        assert_eq!(summary.passed_count, 5);
        assert_eq!(summary.total_stages, 5);
        assert_eq!(summary.pass_rate, 100.0);
    }
}
