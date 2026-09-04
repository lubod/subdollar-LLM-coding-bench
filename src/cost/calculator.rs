use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub prompt_per_million: f64,
    pub completion_per_million: f64,
    pub cache_read_per_million: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub total_cost_usd: f64,
    pub un_cached_cost_usd: f64,
    pub savings_usd: f64,
    pub savings_percent: f64,
}

impl ModelPricing {
    pub fn for_model(model: &str) -> Self {
        let m = model.to_lowercase();
        if m.contains("gemini-2.5-flash") || m.contains("gemini-2.0-flash") || m.contains("gemini-1.5-flash") {
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
        } else if m.contains("qwen-2.5-coder") || m.contains("qwen") {
            ModelPricing {
                prompt_per_million: 0.06,
                completion_per_million: 0.15,
                cache_read_per_million: 0.015, // 75% discount
            }
        } else if m.contains("llama-3.3-70b") || m.contains("llama") {
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
        } else if m.contains("haiku") {
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
        let cached = cached_tokens.min(prompt_tokens);
        let fresh = prompt_tokens.saturating_sub(cached);

        let fresh_cost = (fresh as f64 / 1_000_000.0) * self.prompt_per_million;
        let cached_cost = (cached as f64 / 1_000_000.0) * self.cache_read_per_million;
        let comp_cost = (completion_tokens as f64 / 1_000_000.0) * self.completion_per_million;

        let total_cost_usd = fresh_cost + cached_cost + comp_cost;
        let un_cached_cost_usd =
            (prompt_tokens as f64 / 1_000_000.0) * self.prompt_per_million + comp_cost;

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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(4))
            .build()
            .ok()?;

        let resp = client
            .get("https://openrouter.ai/api/v1/auth/key")
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
