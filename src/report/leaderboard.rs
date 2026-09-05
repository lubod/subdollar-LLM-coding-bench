use anyhow::Result;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, Row, Table};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub use crate::verifier::StageResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRunResult {
    pub id: String,
    pub model: String,
    pub task: String,
    pub language: String,
    #[serde(default)]
    pub effort: Option<String>,
    pub pass_rate: f64,
    pub passed_stages: u32,
    pub total_stages: u32,
    pub throughput_req_sec: Option<f64>,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub total_cost_usd: f64,
    pub savings_percent: f64,
    pub efficiency_score: f64,
    pub timestamp: String,
}

/// Unbiased estimator for Pass@k (Chen et al. 2021)
/// n: total trials, c: number of passing trials, k: target k
pub fn compute_pass_at_k(n: usize, c: usize, k: usize) -> f64 {
    if n == 0 || k == 0 || k > n {
        return 0.0;
    }
    if c >= n {
        return 1.0;
    }
    if c == 0 {
        return 0.0;
    }
    if n - c < k {
        return 1.0;
    }

    let mut prod = 1.0;
    for i in 0..k {
        prod *= (n - c - i) as f64 / (n - i) as f64;
    }
    (1.0 - prod).clamp(0.0, 1.0)
}

pub struct LeaderboardManager;

impl LeaderboardManager {
    pub fn detect_language(workdir: &Path) -> String {
        let mut counts = std::collections::HashMap::new();
        Self::count_lang_files_recursive(workdir, 0, &mut counts);
        counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(lang, _)| lang.to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn count_lang_files_recursive(dir: &Path, depth: u32, counts: &mut std::collections::HashMap<&'static str, usize>) {
        if depth > 4 {
            return;
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }

                if path.is_dir() {
                    Self::count_lang_files_recursive(&path, depth + 1, counts);
                } else if path.is_file() {
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                    match ext {
                        "rs" => *counts.entry("Rust").or_insert(0) += 2,
                        "go" => *counts.entry("Go").or_insert(0) += 2,
                        "py" => *counts.entry("Python").or_insert(0) += 2,
                        "js" | "mjs" | "cjs" => *counts.entry("Node.js").or_insert(0) += 2,
                        "ts" => *counts.entry("TypeScript").or_insert(0) += 2,
                        "c" | "cpp" | "cc" | "h" | "hpp" => *counts.entry("C/C++").or_insert(0) += 2,
                        _ => {}
                    }

                    if name == "Cargo.toml" { *counts.entry("Rust").or_insert(0) += 5; }
                    if name == "go.mod" { *counts.entry("Go").or_insert(0) += 5; }
                    if name == "package.json" { *counts.entry("Node.js").or_insert(0) += 5; }
                    if name == "requirements.txt" || name == "pyproject.toml" { *counts.entry("Python").or_insert(0) += 5; }
                }
            }
        }
    }

    pub fn save_result(results_dir: &str, result: &BenchmarkRunResult) -> Result<()> {
        fs::create_dir_all(results_dir)?;
        let path = Path::new(results_dir).join(format!("{}.json", result.id));
        let data = serde_json::to_string_pretty(result)?;
        fs::write(path, data)?;
        Ok(())
    }

    pub fn load_all(results_dir: &str) -> Vec<BenchmarkRunResult> {
        let mut list = Vec::new();
        let path = Path::new(results_dir);
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(data) = fs::read_to_string(p) {
                        if let Ok(res) = serde_json::from_str::<BenchmarkRunResult>(&data) {
                            list.push(res);
                        }
                    }
                }
            }
        }
        list.sort_by(|a, b| b.efficiency_score.partial_cmp(&a.efficiency_score).unwrap_or(std::cmp::Ordering::Equal));
        list
    }

    pub fn print_table(results: &[BenchmarkRunResult]) {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                "Model",
                "Effort",
                "Task",
                "Lang",
                "Pass Rate",
                "Throughput",
                "Cost ($)",
                "Cache Savings",
                "Score / ¢",
            ]);

        for r in results {
            let tp = r
                .throughput_req_sec
                .map(|t| format!("{:.0} req/s", t))
                .unwrap_or_else(|| "N/A".to_string());

            let pass_str = format!("{:.0}% ({}/{})", r.pass_rate, r.passed_stages, r.total_stages);
            let savings_str = format!("{:.0}%", r.savings_percent);
            let eff_str = r.effort.clone().unwrap_or_else(|| "auto".to_string());

            table.add_row(Row::from(vec![
                Cell::new(&r.model).fg(Color::Cyan),
                Cell::new(eff_str).fg(Color::Yellow),
                Cell::new(&r.task),
                Cell::new(&r.language).fg(Color::Green),
                Cell::new(pass_str).fg(if r.pass_rate == 100.0 { Color::Green } else { Color::Yellow }),
                Cell::new(tp),
                Cell::new(format!("${:.4}", r.total_cost_usd)).fg(Color::Magenta),
                Cell::new(savings_str).fg(Color::Green),
                Cell::new(format!("{:.1}", r.efficiency_score)).fg(Color::Cyan),
            ]));
        }

        println!("\n{}", table);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_pass_at_k() {
        assert_eq!(compute_pass_at_k(0, 0, 1), 0.0);
        assert_eq!(compute_pass_at_k(5, 0, 1), 0.0);
        assert_eq!(compute_pass_at_k(5, 5, 1), 1.0);
        assert_eq!(compute_pass_at_k(5, 5, 5), 1.0);
        assert_eq!(compute_pass_at_k(3, 1, 3), 1.0);
        let p1 = compute_pass_at_k(3, 1, 1);
        assert!((p1 - 0.3333).abs() < 0.01);
    }

    #[test]
    fn test_language_detection() {
        let temp = std::env::temp_dir().join(format!("test_lang_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);

        // Go
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("main.go"), "package main").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Go");
        let _ = fs::remove_dir_all(&temp);

        // Rust nested
        fs::create_dir_all(temp.join("rust-pkg/src")).unwrap();
        fs::write(temp.join("rust-pkg/Cargo.toml"), "[package]").unwrap();
        fs::write(temp.join("rust-pkg/src/main.rs"), "fn main() {}").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Rust");
        let _ = fs::remove_dir_all(&temp);

        // Python
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("server.py"), "print('hi')").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Python");
        let _ = fs::remove_dir_all(&temp);

        // Node
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("index.js"), "console.log(1)").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Node.js");
        let _ = fs::remove_dir_all(&temp);

        // Empty dir
        fs::create_dir_all(&temp).unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Unknown");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_language_detection_c_and_more() {
        let temp = std::env::temp_dir().join(format!("test_c_lang_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);

        // C/C++
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("main.c"), "int main() {}").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "C/C++");
        let _ = fs::remove_dir_all(&temp);

        // Go with go.mod
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("go.mod"), "module test").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Go");
        let _ = fs::remove_dir_all(&temp);

        // Python with requirements.txt
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("requirements.txt"), "flask").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Python");
        let _ = fs::remove_dir_all(&temp);

        // Rust with Cargo.toml
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(LeaderboardManager::detect_language(&temp), "Rust");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_print_table_runs_without_panic() {
        let r = BenchmarkRunResult {
            id: "run_test".to_string(),
            model: "model_test".to_string(),
            task: "redis".to_string(),
            language: "Rust".to_string(),
            effort: Some("auto".to_string()),
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            throughput_req_sec: Some(60000.0),
            prompt_tokens: 1000,
            cached_tokens: 500,
            completion_tokens: 100,
            total_cost_usd: 0.005,
            savings_percent: 50.0,
            efficiency_score: 200.0,
            timestamp: "2026-09-04T12:00:00Z".to_string(),
        };
        LeaderboardManager::print_table(&[r]);
        LeaderboardManager::print_table(&[]);
    }

    #[test]
    fn test_leaderboard_sorting_and_persistence() {
        let temp = std::env::temp_dir().join(format!("test_lb_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);

        let r1 = BenchmarkRunResult {
            id: "run_low_score".to_string(),
            model: "model_b".to_string(),
            task: "redis".to_string(),
            language: "Python".to_string(),
            effort: Some("low".to_string()),
            pass_rate: 50.0,
            passed_stages: 2,
            total_stages: 4,
            throughput_req_sec: None,
            prompt_tokens: 1000,
            cached_tokens: 500,
            completion_tokens: 200,
            total_cost_usd: 0.01,
            savings_percent: 25.0,
            efficiency_score: 50.0,
            timestamp: "2026-09-04T06:00:00Z".to_string(),
        };

        let r2 = BenchmarkRunResult {
            id: "run_high_score".to_string(),
            model: "model_a".to_string(),
            task: "redis".to_string(),
            language: "Go".to_string(),
            effort: Some("high".to_string()),
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            throughput_req_sec: Some(50000.0),
            prompt_tokens: 2000,
            cached_tokens: 1500,
            completion_tokens: 300,
            total_cost_usd: 0.005,
            savings_percent: 60.0,
            efficiency_score: 200.0,
            timestamp: "2026-09-04T06:05:00Z".to_string(),
        };

        LeaderboardManager::save_result(temp.to_str().unwrap(), &r1).unwrap();
        LeaderboardManager::save_result(temp.to_str().unwrap(), &r2).unwrap();

        let list = LeaderboardManager::load_all(temp.to_str().unwrap());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "run_high_score");
        assert_eq!(list[1].id, "run_low_score");

        let _ = fs::remove_dir_all(&temp);
    }
}
