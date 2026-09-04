use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub prompt_per_million: f64,
    pub completion_per_million: f64,
}

impl ModelPricing {
    pub fn for_model(model: &str) -> Self {
        let m = model.to_lowercase();
        if m.contains("gemini-2.5-flash") || m.contains("gemini-2.0-flash") || m.contains("gemini-1.5-flash") {
            ModelPricing { prompt_per_million: 0.15, completion_per_million: 0.60 }
        } else if m.contains("deepseek-chat") || m.contains("deepseek-v3") {
            ModelPricing { prompt_per_million: 0.14, completion_per_million: 0.28 }
        } else if m.contains("qwen-2.5-coder") {
            ModelPricing { prompt_per_million: 0.06, completion_per_million: 0.15 }
        } else if m.contains("llama-3.3-70b") {
            ModelPricing { prompt_per_million: 0.12, completion_per_million: 0.30 }
        } else if m.contains("gpt-4o-mini") {
            ModelPricing { prompt_per_million: 0.15, completion_per_million: 0.60 }
        } else if m.contains("haiku") {
            ModelPricing { prompt_per_million: 0.80, completion_per_million: 4.00 }
        } else if m.contains("mistral-small") {
            ModelPricing { prompt_per_million: 0.10, completion_per_million: 0.30 }
        } else {
            // Default conservative budget estimate
            ModelPricing { prompt_per_million: 0.20, completion_per_million: 0.60 }
        }
    }

    pub fn compute_cost(&self, prompt_tokens: u64, completion_tokens: u64) -> f64 {
        let p_cost = (prompt_tokens as f64 / 1_000_000.0) * self.prompt_per_million;
        let c_cost = (completion_tokens as f64 / 1_000_000.0) * self.completion_per_million;
        p_cost + c_cost
    }
}
