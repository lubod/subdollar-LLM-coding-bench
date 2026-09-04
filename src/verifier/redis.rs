use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

use super::StageResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisTestSummary {
    pub stages: Vec<StageResult>,
    pub passed_count: u32,
    pub total_stages: u32,
    pub pass_rate: f64,
}

pub struct RedisVerifier {
    pub target_port: u16,
    pub _ref_port: Option<u16>,
}

impl RedisVerifier {
    pub fn new(target_port: u16, ref_port: Option<u16>) -> Self {
        Self { target_port, _ref_port: ref_port }
    }

    pub fn format_resp_cmd(args: &[&str]) -> String {
        let mut payload = format!("*{}\r\n", args.len());
        for arg in args {
            payload.push_str(&format!("${}\r\n{}\r\n", arg.len(), arg));
        }
        payload
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
        let payload = Self::format_resp_cmd(args);

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

    pub async fn test_stage1_handshake(&self) -> StageResult {
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

    pub async fn test_stage2_key_value(&self) -> StageResult {
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

    pub async fn test_stage3_expiration(&self) -> StageResult {
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

    pub async fn test_stage4_counters(&self) -> StageResult {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn test_resp_formatting() {
        assert_eq!(RedisVerifier::format_resp_cmd(&["PING"]), "*1\r\n$4\r\nPING\r\n");
        assert_eq!(
            RedisVerifier::format_resp_cmd(&["ECHO", "hello"]),
            "*2\r\n$4\r\nECHO\r\n$5\r\nhello\r\n"
        );
        assert_eq!(
            RedisVerifier::format_resp_cmd(&["SET", "k", "v", "PX", "100"]),
            "*5\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n$2\r\nPX\r\n$3\r\n100\r\n"
        );
    }

    #[tokio::test]
    async fn test_mock_redis_handshake_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];

                if let Ok(n) = socket.read(&mut buf).await {
                    let req = String::from_utf8_lossy(&buf[..n]);
                    if req.contains("PING") {
                        let _ = socket.write_all(b"+PONG\r\n").await;
                    }
                }

                if let Ok(n) = socket.read(&mut buf).await {
                    let req = String::from_utf8_lossy(&buf[..n]);
                    if req.contains("ECHO") {
                        let _ = socket.write_all(b"$14\r\nsubdollar_test\r\n").await;
                    }
                }
            }
        });

        let verifier = RedisVerifier::new(port, None);
        let res = verifier.test_stage1_handshake().await;
        assert!(res.passed, "Stage 1 should pass with mock server: {:?}", res.error);
    }

    #[tokio::test]
    async fn test_mock_redis_connection_refused() {
        let verifier = RedisVerifier::new(59999, None);
        let res = verifier.test_stage1_handshake().await;
        assert!(!res.passed);
        assert!(res.error.is_some());
    }

    #[tokio::test]
    async fn test_mock_redis_run_all_stages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let mut stage2_deleted = false;
                    let mut stage3_count = 0;
                    let mut counter = 0i64;

                    while let Ok(n) = socket.read(&mut buf).await {
                        if n == 0 { break; }
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let upper = req.to_uppercase();

                        if upper.contains("PING") {
                            let _ = socket.write_all(b"+PONG\r\n").await;
                        } else if upper.contains("ECHO") {
                            let _ = socket.write_all(b"$14\r\nsubdollar_test\r\n").await;
                        } else if upper.contains("SET") && upper.contains("PX") {
                            let _ = socket.write_all(b"+OK\r\n").await;
                        } else if upper.contains("SET") {
                            let _ = socket.write_all(b"+OK\r\n").await;
                        } else if upper.contains("GET") && upper.contains("TTL_KEY") {
                            stage3_count += 1;
                            if stage3_count == 1 {
                                let _ = socket.write_all(b"$7\r\nttl_val\r\n").await;
                            } else {
                                let _ = socket.write_all(b"$-1\r\n").await;
                            }
                        } else if upper.contains("GET") && upper.contains("BENCH_KEY") {
                            if !stage2_deleted {
                                let _ = socket.write_all(b"$11\r\nbench_value\r\n").await;
                            } else {
                                let _ = socket.write_all(b"$-1\r\n").await;
                            }
                        } else if upper.contains("EXISTS") && upper.contains("BENCH_KEY") {
                            if !stage2_deleted {
                                let _ = socket.write_all(b":1\r\n").await;
                            } else {
                                let _ = socket.write_all(b":0\r\n").await;
                            }
                        } else if upper.contains("DEL") && upper.contains("BENCH_KEY") {
                            stage2_deleted = true;
                            let _ = socket.write_all(b":1\r\n").await;
                        } else if upper.contains("INCR") && upper.contains("NUM_COUNTER") {
                            counter += 1;
                            let _ = socket.write_all(format!(":{}\r\n", counter).as_bytes()).await;
                        } else if upper.contains("DECR") && upper.contains("NUM_COUNTER") {
                            counter -= 1;
                            let _ = socket.write_all(format!(":{}\r\n", counter).as_bytes()).await;
                        } else {
                            let _ = socket.write_all(b"+OK\r\n").await;
                        }
                    }
                });
            }
        });

        let verifier = RedisVerifier::new(port, None);
        let summary = verifier.run_all().await;
        assert_eq!(summary.passed_count, 4, "Summary failed: {:?}", summary.stages);
        assert_eq!(summary.pass_rate, 100.0);
    }
}
