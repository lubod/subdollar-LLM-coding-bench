use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::report::archive::RunManifest;
use crate::report::environment::EnvironmentInfo;
use crate::report::leaderboard::{BenchmarkRunResult, LeaderboardManager};
use crate::report::summary::SummaryGenerator;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub success: bool,
    pub run_id: String,
    pub commit_hash: String,
    pub commit_message: String,
    pub summary_path: String,
    pub files_committed: Vec<String>,
}

pub struct RunPublisher;

impl RunPublisher {
    pub fn publish_run(
        repo_root: &Path,
        runs_dir: &Path,
        results_dir: &Path,
        run_id: &str,
        custom_message: Option<&str>,
    ) -> Result<PublishResult> {
        let clean_id = Path::new(run_id)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("Invalid run ID: {}", run_id))?;

        let run_dir = runs_dir.join(clean_id);
        if !run_dir.exists() {
            return Err(anyhow!("Run directory not found: {}", run_dir.display()));
        }

        let manifest_path = run_dir.join("manifest.json");
        if !manifest_path.exists() {
            return Err(anyhow!("Manifest not found for run {}", clean_id));
        }

        let manifest_content = fs::read_to_string(&manifest_path)?;
        let mut manifest: RunManifest = serde_json::from_str(&manifest_content)?;

        // 1. Snapshot Environment
        let env = EnvironmentInfo::detect();
        manifest.env = Some(env.clone());
        manifest.git_commit = Some(env.git_commit.clone());
        manifest.is_published = Some(true);

        // 2. Write updated manifest.json
        fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

        // 3. Write env.json
        fs::write(run_dir.join("env.json"), serde_json::to_string_pretty(&env)?)?;

        // 4. Write runs/<run_id>/README.md (individual run report)
        let run_readme = Self::generate_run_readme(&manifest, &env);
        fs::write(run_dir.join("README.md"), run_readme)?;

        // 5. Save / update results/<run_id>.json
        let benchmark_result = BenchmarkRunResult {
            id: manifest.run_id.clone(),
            model: manifest.model.clone(),
            task: manifest.task.clone(),
            language: manifest.language.clone(),
            effort: manifest.effort.clone(),
            pass_rate: manifest.pass_rate,
            passed_stages: manifest.passed_stages,
            total_stages: manifest.total_stages,
            throughput_req_sec: manifest.throughput_req_sec,
            prompt_tokens: manifest.tokens.prompt_tokens,
            cached_tokens: manifest.tokens.cached_tokens,
            completion_tokens: manifest.tokens.completion_tokens,
            total_cost_usd: manifest.cost_usd,
            savings_percent: manifest.savings_percent,
            efficiency_score: manifest.efficiency_score,
            timestamp: manifest.completed_at.clone(),
        };
        let _ = LeaderboardManager::save_result(results_dir.to_str().unwrap_or("./results"), &benchmark_result);

        // 6. Regenerate SUMMARY.md
        let summary_path = SummaryGenerator::update_summary_file(repo_root, runs_dir)?;

        // 7. Commit to Git
        let commit_msg = if let Some(m) = custom_message {
            m.to_string()
        } else {
            let tp_str = manifest
                .throughput_req_sec
                .map(|t| format!("{:.0} req/s", t))
                .unwrap_or_else(|| "N/A".to_string());
            format!(
                "bench: record {} result for {} (lang: {}, pass: {:.0}%, tp: {}, score: {:.1} pts/¢)",
                manifest.task,
                manifest.model,
                manifest.language,
                manifest.pass_rate,
                tp_str,
                manifest.efficiency_score
            )
        };

        let run_rel_path = format!("runs/{}", clean_id);
        let res_rel_path = format!("results/{}.json", clean_id);

        let files_to_add = vec![
            run_rel_path.clone(),
            res_rel_path.clone(),
            "SUMMARY.md".to_string(),
        ];

        // git add
        let mut add_cmd = Command::new("git");
        add_cmd.current_dir(repo_root).arg("add");
        for f in &files_to_add {
            add_cmd.arg(f);
        }
        let _ = add_cmd.output();

        // git commit
        let _ = Command::new("git")
            .current_dir(repo_root)
            .args(["commit", "-m", &commit_msg])
            .output();

        // Read current commit hash
        let hash_output = Command::new("git")
            .current_dir(repo_root)
            .args(["rev-parse", "--short", "HEAD"])
            .output();
        let commit_hash = hash_output
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(PublishResult {
            success: true,
            run_id: clean_id.to_string(),
            commit_hash,
            commit_message: commit_msg,
            summary_path: summary_path.to_string_lossy().to_string(),
            files_committed: files_to_add,
        })
    }

    fn generate_run_readme(m: &RunManifest, env: &EnvironmentInfo) -> String {
        let mut md = String::new();
        md.push_str(&format!("# Benchmark Run: `{}`\n\n", m.run_id));
        md.push_str(&format!("- **Model**: `{}`\n", m.model));
        md.push_str(&format!("- **Task**: `{}`\n", m.task));
        md.push_str(&format!("- **Language Detected**: `{}`\n", m.language));
        md.push_str(&format!("- **Reasoning Effort**: `{}`\n", m.effort.as_deref().unwrap_or("auto")));
        md.push_str(&format!("- **Pass Rate**: {:.1}% ({}/{} stages)\n", m.pass_rate, m.passed_stages, m.total_stages));
        if let Some(tp) = m.throughput_req_sec {
            md.push_str(&format!("- **Throughput**: {:.0} req/sec\n", tp));
        }
        md.push_str(&format!("- **Cost**: ${:.4} USD\n", m.cost_usd));
        md.push_str(&format!("- **Cache Savings**: {:.1}%\n", m.savings_percent));
        md.push_str(&format!("- **Efficiency Score**: **{:.1} pts/¢**\n", m.efficiency_score));
        md.push_str(&format!("- **Execution Duration**: {:.1}s\n\n", m.duration_seconds));

        md.push_str("## 🧪 Verification Stages\n\n");
        md.push_str("| Stage | Name | Status | Error |\n");
        md.push_str("|:---:|:---|:---:|:---|\n");
        for s in &m.stages {
            let status = if s.passed { "✅ PASS" } else { "❌ FAIL" };
            let err = s.error.as_deref().unwrap_or("-");
            md.push_str(&format!("| {} | {} | {} | {} |\n", s.stage, s.name, status, err));
        }
        md.push_str("\n");

        md.push_str("## 📁 Generated Project Files\n\n");
        md.push_str("Candidate code snapshot in [`workspace/`](workspace/):\n\n");
        md.push_str("| File | Size (Bytes) |\n");
        md.push_str("|:---|:---:|\n");
        for f in &m.files {
            md.push_str(&format!("| [`{}`](workspace/{}) | {} |\n", f.name, f.name, f.size_bytes));
        }
        md.push_str("\n");

        md.push_str("## 🖥️ Execution Environment\n\n");
        md.push_str(&env.to_markdown_table());
        md.push_str("\n\n");

        md.push_str("## 📜 Full Console Audit Log\n\n");
        md.push_str("See [`console.log`](console.log) for the complete turn-by-turn log of thoughts, tool actions, and compiler/harness outputs.\n");

        md
    }
}
