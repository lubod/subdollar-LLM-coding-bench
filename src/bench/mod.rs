use anyhow::{anyhow, Result};
use regex::Regex;
use std::process::Command;

pub struct BenchmarkRunner;

impl BenchmarkRunner {
    pub fn run_redis_benchmark(port: u16) -> Result<f64> {
        let output = Command::new("docker")
            .args([
                "run",
                "--rm",
                "--network",
                "host",
                "redis:alpine",
                "redis-benchmark",
                "-h",
                "127.0.0.1",
                "-p",
                &port.to_string(),
                "-n",
                "5000",
                "-c",
                "10",
                "-t",
                "get,set",
                "-q",
            ])
            .output()
            .map_err(|e| anyhow!("Failed to execute docker redis-benchmark: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let re = Regex::new(r"(\d+\.?\d*)\s+requests per second").unwrap();

        let mut rates = Vec::new();
        for cap in re.captures_iter(&stdout) {
            if let Some(m) = cap.get(1) {
                if let Ok(val) = m.as_str().parse::<f64>() {
                    rates.push(val);
                }
            }
        }

        if rates.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Could not parse redis-benchmark output. stdout: '{}', stderr: '{}'", stdout, stderr));
        }

        let avg = rates.iter().sum::<f64>() / rates.len() as f64;
        Ok(avg)
    }

    pub fn run_wrk_benchmark(port: u16) -> Result<f64> {
        let url = format!("http://127.0.0.1:{}/", port);
        let output = Command::new("docker")
            .args([
                "run",
                "--rm",
                "--network",
                "host",
                "williamyeh/wrk",
                "-t2",
                "-c20",
                "-d3s",
                &url,
            ])
            .output()
            .map_err(|e| anyhow!("Failed to execute docker wrk: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let re = Regex::new(r"Requests/sec:\s+(\d+\.?\d*)").unwrap();

        if let Some(cap) = re.captures(&stdout) {
            if let Some(m) = cap.get(1) {
                if let Ok(val) = m.as_str().parse::<f64>() {
                    return Ok(val);
                }
            }
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!("Could not parse wrk output. stdout: '{}', stderr: '{}'", stdout, stderr))
    }
}
