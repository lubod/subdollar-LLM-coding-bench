use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub total_cost_usd: f64,
    pub un_cached_cost_usd: f64,
    pub savings_usd: f64,
    pub savings_percent: f64,
}

#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub prompt_per_million: f64,
    pub completion_per_million: f64,
    pub cache_read_per_million: f64,
}

impl ModelPricing {
    pub fn new(
        prompt_per_million: f64,
        completion_per_million: f64,
        cache_read_per_million: f64,
    ) -> Self {
        Self {
            prompt_per_million,
            completion_per_million,
            cache_read_per_million,
        }
    }

    pub fn from_rates(prompt_per_m: f64, completion_per_m: f64) -> Self {
        Self {
            prompt_per_million: prompt_per_m,
            completion_per_million: completion_per_m,
            cache_read_per_million: prompt_per_m * 0.25,
        }
    }

    pub fn for_model(model_name: &str) -> Self {
        let m = model_name.to_lowercase();
        if m.contains("gemini-2.5-flash") {
            ModelPricing {
                prompt_per_million: 0.15,
                completion_per_million: 0.60,
                cache_read_per_million: 0.0375, // 75% discount
            }
        } else if m.contains("deepseek-chat") || m.contains("deepseek-v3") {
            ModelPricing {
                prompt_per_million: 0.14,
                completion_per_million: 0.28,
                cache_read_per_million: 0.014, // 90% discount
            }
        } else if m.contains("qwen-2.5-coder-32b") {
            ModelPricing {
                prompt_per_million: 0.06,
                completion_per_million: 0.15,
                cache_read_per_million: 0.015, // 75% discount
            }
        } else if m.contains("llama-3.3-70b") {
            ModelPricing {
                prompt_per_million: 0.12,
                completion_per_million: 0.30,
                cache_read_per_million: 0.030, // 75% discount
            }
        } else if m.contains("gpt-4o-mini") {
            ModelPricing {
                prompt_per_million: 0.15,
                completion_per_million: 0.60,
                cache_read_per_million: 0.075, // 50% discount
            }
        } else if m.contains("claude-3-haiku") {
            ModelPricing {
                prompt_per_million: 0.80,
                completion_per_million: 4.00,
                cache_read_per_million: 0.08, // 90% discount
            }
        } else if m.contains("mistral-small") {
            ModelPricing {
                prompt_per_million: 0.10,
                completion_per_million: 0.30,
                cache_read_per_million: 0.025, // 75% discount
            }
        } else {
            ModelPricing {
                prompt_per_million: 0.20,
                completion_per_million: 0.60,
                cache_read_per_million: 0.050, // 75% discount fallback
            }
        }
    }

    pub fn compute_cost_with_cache(
        &self,
        prompt_tokens: u64,
        cached_tokens: u64,
        completion_tokens: u64,
    ) -> CostBreakdown {
        let fresh_cost = (prompt_tokens as f64 / 1_000_000.0) * self.prompt_per_million;
        let cached_cost = (cached_tokens as f64 / 1_000_000.0) * self.cache_read_per_million;
        let comp_cost = (completion_tokens as f64 / 1_000_000.0) * self.completion_per_million;

        let total_cost_usd = fresh_cost + cached_cost + comp_cost;
        let un_cached_cost_usd =
            ((prompt_tokens + cached_tokens) as f64 / 1_000_000.0) * self.prompt_per_million + comp_cost;

        let savings_usd = (un_cached_cost_usd - total_cost_usd).max(0.0);
        let savings_percent = if un_cached_cost_usd > 0.0 {
            (savings_usd / un_cached_cost_usd) * 100.0
        } else {
            0.0
        };

        CostBreakdown {
            total_cost_usd,
            un_cached_cost_usd,
            savings_usd,
            savings_percent,
        }
    }

    /// Query OpenRouter /api/v1/auth/key to get the exact live cumulative dollar usage
    pub async fn query_openrouter_key_usage(api_key: &str) -> Option<f64> {
        Self::query_openrouter_key_usage_at(api_key, "https://openrouter.ai/api/v1/auth/key").await
    }

    pub async fn query_openrouter_key_usage_at(api_key: &str, url: &str) -> Option<f64> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(4))
            .build()
            .ok()?;

        let resp = client
            .get(url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let body: serde_json::Value = resp.json().await.ok()?;
        body.get("data")?
            .get("usage")?
            .as_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn test_pricing_for_known_models() {
        let gemini = ModelPricing::for_model("google/gemini-2.5-flash");
        assert_eq!(gemini.prompt_per_million, 0.15);
        assert_eq!(gemini.completion_per_million, 0.60);
        assert_eq!(gemini.cache_read_per_million, 0.0375);

        let deepseek = ModelPricing::for_model("deepseek/deepseek-chat");
        assert_eq!(deepseek.prompt_per_million, 0.14);
        assert_eq!(deepseek.completion_per_million, 0.28);
        assert_eq!(deepseek.cache_read_per_million, 0.014);

        let qwen = ModelPricing::for_model("qwen/qwen-2.5-coder-32b-instruct");
        assert_eq!(qwen.prompt_per_million, 0.06);
        assert_eq!(qwen.completion_per_million, 0.15);
        assert_eq!(qwen.cache_read_per_million, 0.015);

        let llama = ModelPricing::for_model("meta-llama/llama-3.3-70b-instruct");
        assert_eq!(llama.prompt_per_million, 0.12);

        let gpt4o = ModelPricing::for_model("openai/gpt-4o-mini");
        assert_eq!(gpt4o.prompt_per_million, 0.15);
        assert_eq!(gpt4o.cache_read_per_million, 0.075);

        let haiku = ModelPricing::for_model("anthropic/claude-3-haiku");
        assert_eq!(haiku.prompt_per_million, 0.80);
        assert_eq!(haiku.completion_per_million, 4.00);

        let mistral = ModelPricing::for_model("mistralai/mistral-small");
        assert_eq!(mistral.prompt_per_million, 0.10);
        assert_eq!(mistral.completion_per_million, 0.30);

        let custom = ModelPricing::from_rates(0.50, 1.50);
        assert_eq!(custom.prompt_per_million, 0.50);
        assert_eq!(custom.completion_per_million, 1.50);
        assert_eq!(custom.cache_read_per_million, 0.125);

        let unknown = ModelPricing::for_model("some-random-unknown-model");
        assert_eq!(unknown.prompt_per_million, 0.20);
        assert_eq!(unknown.completion_per_million, 0.60);
    }

    #[test]
    fn test_cost_calculation_zero_tokens() {
        let p = ModelPricing::for_model("google/gemini-2.5-flash");
        let b = p.compute_cost_with_cache(0, 0, 0);
        assert_eq!(b.total_cost_usd, 0.0);
        assert_eq!(b.un_cached_cost_usd, 0.0);
        assert_eq!(b.savings_usd, 0.0);
        assert_eq!(b.savings_percent, 0.0);
    }

    #[test]
    fn test_cost_calculation_no_cache() {
        let p = ModelPricing::for_model("google/gemini-2.5-flash");
        let b = p.compute_cost_with_cache(1_000_000, 0, 1_000_000);
        let diff = (b.total_cost_usd - 0.75).abs();
        assert!(diff < 1e-6, "Expected 0.75, got {}", b.total_cost_usd);
        assert_eq!(b.savings_usd, 0.0);
        assert_eq!(b.savings_percent, 0.0);
    }

    #[test]
    fn test_cost_calculation_full_prompt_cache() {
        let p = ModelPricing::for_model("google/gemini-2.5-flash");
        let b = p.compute_cost_with_cache(0, 1_000_000, 0);
        let diff = (b.total_cost_usd - 0.0375).abs();
        assert!(diff < 1e-6, "Expected 0.0375, got {}", b.total_cost_usd);
        let diff_savings = (b.savings_usd - 0.1125).abs();
        assert!(diff_savings < 1e-6, "Expected 0.1125, got {}", b.savings_usd);
        assert!((b.savings_percent - 75.0).abs() < 1e-4);
    }

    #[test]
    fn test_cost_calculation_cached_exceeds_prompt() {
        let p = ModelPricing::for_model("google/gemini-2.5-flash");
        let b = p.compute_cost_with_cache(500_000, 1_000_000, 0);
        let diff = (b.total_cost_usd - 0.1125).abs();
        assert!(diff < 1e-6, "Expected 0.1125, got {}", b.total_cost_usd);
        assert!((b.savings_percent - 50.0).abs() < 1e-4);
    }

    #[tokio::test]
    async fn test_query_openrouter_key_usage_mock() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let body = r#"{"data":{"usage": 0.4567}}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let url = format!("http://127.0.0.1:{}/api/v1/auth/key", port);
        let usage = ModelPricing::query_openrouter_key_usage_at("test-key", &url).await;
        assert_eq!(usage, Some(0.4567));
    }

    #[tokio::test]
    async fn test_query_openrouter_key_usage_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let url = format!("http://127.0.0.1:{}/api/v1/auth/key", port);
        let usage = ModelPricing::query_openrouter_key_usage_at("bad-key", &url).await;
        assert_eq!(usage, None);
    }
}
