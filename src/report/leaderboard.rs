use anyhow::Result;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, Row, Table};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRunResult {
    pub id: String,
    pub model: String,
    pub task: String,
    pub language: String,
    pub pass_rate: f64,
    pub passed_stages: u32,
    pub total_stages: u32,
    pub throughput_req_sec: Option<f64>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_cost_usd: f64,
    pub efficiency_score: f64,
    pub timestamp: String,
}

pub struct LeaderboardManager;

impl LeaderboardManager {
    pub fn detect_language(workdir: &Path) -> String {
        let mut counts = std::collections::HashMap::new();
        if let Ok(entries) = fs::read_dir(workdir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    match ext {
                        "go" => *counts.entry("Go").or_insert(0) += 1,
                        "py" => *counts.entry("Python").or_insert(0) += 1,
                        "rs" => *counts.entry("Rust").or_insert(0) += 1,
                        "js" | "ts" => *counts.entry("Node.js").or_insert(0) += 1,
                        "c" | "cpp" => *counts.entry("C/C++").or_insert(0) += 1,
                        _ => {}
                    }
                }
            }
        }
        counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(lang, _)| lang.to_string())
            .unwrap_or_else(|| "Unknown".to_string())
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
        list.sort_by(|a, b| b.efficiency_score.partial_cmp(&a.efficiency_score).unwrap());
        list
    }

    pub fn print_table(results: &[BenchmarkRunResult]) {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                "Model",
                "Task",
                "Lang",
                "Pass Rate",
                "Throughput",
                "Cost ($)",
                "Score / ¢",
            ]);

        for r in results {
            let tp = r
                .throughput_req_sec
                .map(|t| format!("{:.0} req/s", t))
                .unwrap_or_else(|| "N/A".to_string());

            let pass_str = format!("{:.0}% ({}/{})", r.pass_rate, r.passed_stages, r.total_stages);

            table.add_row(Row::from(vec![
                Cell::new(&r.model).fg(Color::Cyan),
                Cell::new(&r.task),
                Cell::new(&r.language).fg(Color::Green),
                Cell::new(pass_str).fg(if r.pass_rate == 100.0 { Color::Green } else { Color::Yellow }),
                Cell::new(tp),
                Cell::new(format!("${:.4}", r.total_cost_usd)).fg(Color::Magenta),
                Cell::new(format!("{:.1}", r.efficiency_score)).fg(Color::Cyan),
            ]));
        }

        println!("\n{}", table);
    }

    pub fn generate_markdown(results: &[BenchmarkRunResult]) -> String {
        let mut md = String::from(
            "| Model | Task | Lang | Pass Rate | Throughput | Cost ($) | Efficiency (Score / ¢) |\n\
             | :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n",
        );

        for r in results {
            let tp = r
                .throughput_req_sec
                .map(|t| format!("{:.0} req/s", t))
                .unwrap_or_else(|| "N/A".to_string());

            md.push_str(&format!(
                "| `{}` | {} | {} | {:.0}% ({}/{}) | {} | ${:.4} | **{:.1}** |\n",
                r.model,
                r.task,
                r.language,
                r.pass_rate,
                r.passed_stages,
                r.total_stages,
                tp,
                r.total_cost_usd,
                r.efficiency_score
            ));
        }

        md
    }
}
