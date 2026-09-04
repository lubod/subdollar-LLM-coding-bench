use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageResult {
    pub stage: u32,
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisTestSummary {
    pub stages: Vec<StageResult>,
    pub passed_count: u32,
    pub total_stages: u32,
    pub pass_rate: f64,
}

pub struct RedisVerifier {
    pub target_port: u16,
    pub ref_port: Option<u16>,
}

impl RedisVerifier {
    pub fn new(target_port: u16, ref_port: Option<u16>) -> Self {
        Self { target_port, ref_port }
    }

    pub async fn run_all(&self) -> RedisTestSummary {
        let mut stages = Vec::new();

        stages.push(self.test_stage1_handshake().await);
        stages.push(self.test_stage2_key_value().await);
        stages.push(self.test_stage3_expiration().await);
        stages.push(self.test_stage4_counters().await);

        let passed_count = stages.iter().filter(|s| s.passed).count() as u32;
        let total_stages = stages.len() as u32;
        let pass_rate = (passed_count as f64 / total_stages as f64) * 100.0;

        RedisTestSummary {
            stages,
            passed_count,
            total_stages,
            pass_rate,
        }
    }

    async fn connect(&self) -> Result<TcpStream> {
        let addr = format!("127.0.0.1:{}", self.target_port);
        tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(&addr))
            .await
            .map_err(|_| anyhow!("Connection timed out to {}", addr))?
            .map_err(|e| anyhow!("Failed to connect to {}: {}", addr, e))
    }

    async fn send_resp_cmd(stream: &mut TcpStream, args: &[&str]) -> Result<String> {
        let mut payload = format!("*{}\r\n", args.len());
        for arg in args {
            payload.push_str(&format!("${}\r\n{}\r\n", arg.len(), arg));
        }

        stream.write_all(payload.as_bytes()).await?;
        stream.flush().await?;

        let mut buf = [0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .map_err(|_| anyhow!("Read timed out"))??;

        if n == 0 {
            return Err(anyhow!("Server closed connection prematurely"));
        }

        Ok(String::from_utf8_lossy(&buf[..n]).to_string())
    }

    async fn test_stage1_handshake(&self) -> StageResult {
        let name = "Stage 1: PING & ECHO Handshake".to_string();
        let mut stream = match self.connect().await {
            Ok(s) => s,
            Err(e) => return StageResult { stage: 1, name, passed: false, error: Some(e.to_string()) },
        };

        let res = match Self::send_resp_cmd(&mut stream, &["PING"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 1, name, passed: false, error: Some(format!("PING failed: {}", e)) },
        };

        if !res.contains("PONG") && !res.starts_with("+PONG") {
            return StageResult {
                stage: 1,
                name,
                passed: false,
                error: Some(format!("Expected +PONG\\r\\n, got: {:?}", res)),
            };
        }

        let res = match Self::send_resp_cmd(&mut stream, &["ECHO", "subdollar_test"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 1, name, passed: false, error: Some(format!("ECHO failed: {}", e)) },
        };

        if !res.contains("subdollar_test") {
            return StageResult {
                stage: 1,
                name,
                passed: false,
                error: Some(format!("Expected ECHO with 'subdollar_test', got: {:?}", res)),
            };
        }

        StageResult { stage: 1, name, passed: true, error: None }
    }

    async fn test_stage2_key_value(&self) -> StageResult {
        let name = "Stage 2: Basic Key-Value (SET/GET/DEL/EXISTS)".to_string();
        let mut stream = match self.connect().await {
            Ok(s) => s,
            Err(e) => return StageResult { stage: 2, name, passed: false, error: Some(e.to_string()) },
        };

        let set_res = match Self::send_resp_cmd(&mut stream, &["SET", "bench_key", "bench_value"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 2, name, passed: false, error: Some(format!("SET failed: {}", e)) },
        };
        if !set_res.starts_with("+OK") && !set_res.contains("OK") {
            return StageResult { stage: 2, name, passed: false, error: Some(format!("Expected +OK, got {:?}", set_res)) };
        }

        let get_res = match Self::send_resp_cmd(&mut stream, &["GET", "bench_key"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 2, name, passed: false, error: Some(format!("GET failed: {}", e)) },
        };
        if !get_res.contains("bench_value") {
            return StageResult { stage: 2, name, passed: false, error: Some(format!("Expected 'bench_value', got {:?}", get_res)) };
        }

        let exists_res = match Self::send_resp_cmd(&mut stream, &["EXISTS", "bench_key"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 2, name, passed: false, error: Some(format!("EXISTS failed: {}", e)) },
        };
        if !exists_res.contains(":1") && !exists_res.contains("1") {
            return StageResult { stage: 2, name, passed: false, error: Some(format!("Expected :1, got {:?}", exists_res)) };
        }

        let del_res = match Self::send_resp_cmd(&mut stream, &["DEL", "bench_key"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 2, name, passed: false, error: Some(format!("DEL failed: {}", e)) },
        };
        if !del_res.contains(":1") && !del_res.contains("1") {
            return StageResult { stage: 2, name, passed: false, error: Some(format!("Expected :1 from DEL, got {:?}", del_res)) };
        }

        let get_nil = match Self::send_resp_cmd(&mut stream, &["GET", "bench_key"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 2, name, passed: false, error: Some(format!("GET after DEL failed: {}", e)) },
        };
        if !get_nil.contains("$-1") {
            return StageResult { stage: 2, name, passed: false, error: Some(format!("Expected nil ($-1\\r\\n), got {:?}", get_nil)) };
        }

        StageResult { stage: 2, name, passed: true, error: None }
    }

    async fn test_stage3_expiration(&self) -> StageResult {
        let name = "Stage 3: Expiration (SET ... PX <ms>)".to_string();
        let mut stream = match self.connect().await {
            Ok(s) => s,
            Err(e) => return StageResult { stage: 3, name, passed: false, error: Some(e.to_string()) },
        };

        let set_res = match Self::send_resp_cmd(&mut stream, &["SET", "ttl_key", "ttl_val", "PX", "150"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 3, name, passed: false, error: Some(format!("SET PX failed: {}", e)) },
        };
        if !set_res.contains("OK") {
            return StageResult { stage: 3, name, passed: false, error: Some(format!("Expected +OK for SET PX, got {:?}", set_res)) };
        }

        let get_fast = match Self::send_resp_cmd(&mut stream, &["GET", "ttl_key"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 3, name, passed: false, error: Some(format!("Immediate GET failed: {}", e)) },
        };
        if !get_fast.contains("ttl_val") {
            return StageResult { stage: 3, name, passed: false, error: Some(format!("Expected immediate GET to return ttl_val, got {:?}", get_fast)) };
        }

        sleep(Duration::from_millis(250)).await;

        let get_expired = match Self::send_resp_cmd(&mut stream, &["GET", "ttl_key"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 3, name, passed: false, error: Some(format!("Post-TTL GET failed: {}", e)) },
        };
        if !get_expired.contains("$-1") {
            return StageResult { stage: 3, name, passed: false, error: Some(format!("Expected expired key to return nil ($-1\\r\\n), got {:?}", get_expired)) };
        }

        StageResult { stage: 3, name, passed: true, error: None }
    }

    async fn test_stage4_counters(&self) -> StageResult {
        let name = "Stage 4: INCR & DECR Arithmetic".to_string();
        let mut stream = match self.connect().await {
            Ok(s) => s,
            Err(e) => return StageResult { stage: 4, name, passed: false, error: Some(e.to_string()) },
        };

        let incr1 = match Self::send_resp_cmd(&mut stream, &["INCR", "num_counter"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 4, name, passed: false, error: Some(format!("INCR 1 failed: {}", e)) },
        };
        if !incr1.contains(":1") {
            return StageResult { stage: 4, name, passed: false, error: Some(format!("Expected :1, got {:?}", incr1)) };
        }

        let incr2 = match Self::send_resp_cmd(&mut stream, &["INCR", "num_counter"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 4, name, passed: false, error: Some(format!("INCR 2 failed: {}", e)) },
        };
        if !incr2.contains(":2") {
            return StageResult { stage: 4, name, passed: false, error: Some(format!("Expected :2, got {:?}", incr2)) };
        }

        let decr1 = match Self::send_resp_cmd(&mut stream, &["DECR", "num_counter"]).await {
            Ok(r) => r,
            Err(e) => return StageResult { stage: 4, name, passed: false, error: Some(format!("DECR failed: {}", e)) },
        };
        if !decr1.contains(":1") {
            return StageResult { stage: 4, name, passed: false, error: Some(format!("Expected :1, got {:?}", decr1)) };
        }

        StageResult { stage: 4, name, passed: true, error: None }
    }
}
