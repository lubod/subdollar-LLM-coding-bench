use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageResult {
    pub stage: u32,
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

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

    async fn test_stage1_root(&self) -> StageResult {
        let name = "Stage 1: GET / (Root 200 OK)".to_string();
        let url = format!("http://127.0.0.1:{}/", self.target_port);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    StageResult { stage: 1, name, passed: true, error: None }
                } else {
                    StageResult { stage: 1, name, passed: false, error: Some(format!("Expected 200 OK, got {}", resp.status())) }
                }
            }
            Err(e) => StageResult { stage: 1, name, passed: false, error: Some(e.to_string()) },
        }
    }

    async fn test_stage2_not_found(&self) -> StageResult {
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

    async fn test_stage3_echo(&self) -> StageResult {
        let name = "Stage 3: GET /echo/{str}".to_string();
        let word = "subdollar_speed_test";
        let url = format!("http://127.0.0.1:{}/echo/{}", self.target_port, word);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return StageResult { stage: 3, name, passed: false, error: Some(format!("Expected 200 OK, got {}", resp.status())) };
                }
                match resp.text().await {
                    Ok(text) if text.contains(word) => StageResult { stage: 3, name, passed: true, error: None },
                    Ok(text) => StageResult { stage: 3, name, passed: false, error: Some(format!("Body didn't match '{}', got: '{}'", word, text)) },
                    Err(e) => StageResult { stage: 3, name, passed: false, error: Some(e.to_string()) },
                }
            }
            Err(e) => StageResult { stage: 3, name, passed: false, error: Some(e.to_string()) },
        }
    }

    async fn test_stage4_user_agent(&self) -> StageResult {
        let name = "Stage 4: GET /user-agent Header Echo".to_string();
        let ua = "SubDollarBench-Agent/1.0";
        let url = format!("http://127.0.0.1:{}/user-agent", self.target_port);
        match self.client.get(&url).header("User-Agent", ua).send().await {
            Ok(resp) => {
                match resp.text().await {
                    Ok(text) if text.contains(ua) => StageResult { stage: 4, name, passed: true, error: None },
                    Ok(text) => StageResult { stage: 4, name, passed: false, error: Some(format!("Expected UA '{}', got '{}'", ua, text)) },
                    Err(e) => StageResult { stage: 4, name, passed: false, error: Some(e.to_string()) },
                }
            }
            Err(e) => StageResult { stage: 4, name, passed: false, error: Some(e.to_string()) },
        }
    }

    async fn test_stage5_file_storage(&self) -> StageResult {
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
                if !resp.status().is_success() {
                    return StageResult { stage: 5, name, passed: false, error: Some(format!("GET /files failed with status: {}", resp.status())) };
                }
                match resp.text().await {
                    Ok(text) if text == content => StageResult { stage: 5, name, passed: true, error: None },
                    Ok(text) => StageResult { stage: 5, name, passed: false, error: Some(format!("File content mismatch! Expected '{}', got '{}'", content, text)) },
                    Err(e) => StageResult { stage: 5, name, passed: false, error: Some(e.to_string()) },
                }
            }
            Err(e) => StageResult { stage: 5, name, passed: false, error: Some(format!("GET error: {}", e)) },
        }
    }
}
