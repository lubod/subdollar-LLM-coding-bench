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
            method_version: manifest.method_version.clone(),
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

        // git add -f to ensure gitignored runs/ and results/ directories are force-staged
        let mut add_cmd = Command::new("git");
        add_cmd.current_dir(repo_root).arg("add").arg("-f");
        for f in &files_to_add {
            add_cmd.arg(f);
        }
        let add_out = add_cmd.output()?;
        if !add_out.status.success() {
            let stderr = String::from_utf8_lossy(&add_out.stderr);
            return Err(anyhow!("git add -f failed: {}", stderr));
        }

        // git commit
        let commit_out = Command::new("git")
            .current_dir(repo_root)
            .args(["commit", "-m", &commit_msg])
            .output()?;
        if !commit_out.status.success() {
            let stdout = String::from_utf8_lossy(&commit_out.stdout);
            let stderr = String::from_utf8_lossy(&commit_out.stderr);
            let combined = format!("{}\n{}", stdout, stderr);
            if !combined.contains("nothing to commit") && !combined.contains("working tree clean") {
                return Err(anyhow!("git commit failed: {}", combined));
            }
        }

        // Read current commit hash
        let hash_output = Command::new("git")
            .current_dir(repo_root)
            .args(["rev-parse", "--short", "HEAD"])
            .output()?;
        let commit_hash = if hash_output.status.success() {
            String::from_utf8_lossy(&hash_output.stdout).trim().to_string()
        } else {
            "unknown".to_string()
        };

        Ok(PublishResult {
            success: true,
            run_id: clean_id.to_string(),
            commit_hash,
            commit_message: commit_msg,
            summary_path: summary_path.to_string_lossy().to_string(),
            files_committed: files_to_add,
        })
    }

    pub fn generate_run_readme(m: &RunManifest, env: &EnvironmentInfo) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::archive::{FileInfo, RunTokenUsage};
    use crate::verifier::StageResult;

    #[test]
    fn test_generate_run_readme_and_publish_result_serde() {
        let env = EnvironmentInfo {
            os: "Ubuntu 24.04 LTS".to_string(),
            kernel: "Linux 6.8.0".to_string(),
            arch: "x86_64".to_string(),
            cpu_model: "Intel Xeon".to_string(),
            cpu_cores: 4,
            total_memory: "8.0 GiB".to_string(),
            docker_version: "Docker 27.1.1".to_string(),
            rust_version: "rustc 1.81.0".to_string(),
            python_version: "Python 3.12".to_string(),
            node_version: "v20.0".to_string(),
            git_commit: "abc1234".to_string(),
            git_branch: "master".to_string(),
            timestamp: "2026-09-04T12:00:00Z".to_string(),
        };

        let manifest = RunManifest {
            run_id: "test_run_1".to_string(),
            model: "test/model".to_string(),
            task: "redis".to_string(),
            status: "completed".to_string(),
            language: "Rust".to_string(),
            effort: Some("max".to_string()),
            started_at: "2026-09-04T11:58:00Z".to_string(),
            completed_at: "2026-09-04T12:00:00Z".to_string(),
            duration_seconds: 120.0,
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            stages: vec![
                StageResult { stage: 1, name: "Handshake".to_string(), passed: true, error: None },
                StageResult { stage: 2, name: "CRUD".to_string(), passed: true, error: None },
            ],
            throughput_req_sec: Some(55000.0),
            tokens: RunTokenUsage { prompt_tokens: 1000, cached_tokens: 500, completion_tokens: 200, total_tokens: 1200 },
            cost_usd: 0.01,
            savings_percent: 50.0,
            efficiency_score: 100.0,
            files: vec![
                FileInfo { name: "main.rs".to_string(), size_bytes: 1234 },
            ],
            env: Some(env.clone()),
            git_commit: Some("abc1234".to_string()),
            is_published: Some(true),
            method_version: Some("0.1.0".to_string()),
        };

        let readme = RunPublisher::generate_run_readme(&manifest, &env);
        assert!(readme.contains("test_run_1"));
        assert!(readme.contains("test/model"));
        assert!(readme.contains("Handshake"));
        assert!(readme.contains("55000 req/sec"));
        assert!(readme.contains("main.rs"));

        let res = PublishResult {
            success: true,
            run_id: "test_run_1".to_string(),
            commit_hash: "1234567".to_string(),
            commit_message: "bench: test commit".to_string(),
            summary_path: "SUMMARY.md".to_string(),
            files_committed: vec!["runs/test_run_1".to_string()],
        };
        let s = serde_json::to_string(&res).unwrap();
        let de: PublishResult = serde_json::from_str(&s).unwrap();
        assert_eq!(de.run_id, "test_run_1");
    }

    #[test]
    fn test_publish_run_git_flow() {
        let temp_dir = std::env::temp_dir().join(format!("test_git_publish_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Initialize git repo
        let _ = Command::new("git").current_dir(&temp_dir).args(["init"]).output();
        let _ = Command::new("git").current_dir(&temp_dir).args(["config", "user.name", "Bench Tester"]).output();
        let _ = Command::new("git").current_dir(&temp_dir).args(["config", "user.email", "tester@bench.test"]).output();

        // Write .gitignore ignoring runs/ and results/ to verify force add
        fs::write(temp_dir.join(".gitignore"), "runs/\nresults/\n").unwrap();
        let _ = Command::new("git").current_dir(&temp_dir).args(["add", ".gitignore"]).output();
        let _ = Command::new("git").current_dir(&temp_dir).args(["commit", "-m", "chore: initial commit"]).output();

        let runs_dir = temp_dir.join("runs");
        let results_dir = temp_dir.join("results");
        let run_id = "test_publish_run_42";
        let run_dir = runs_dir.join(run_id);
        fs::create_dir_all(run_dir.join("workspace")).unwrap();
        fs::write(run_dir.join("workspace/main.rs"), "fn main() {}").unwrap();

        let manifest = RunManifest {
            run_id: run_id.to_string(),
            model: "test/publisher-model".to_string(),
            task: "redis".to_string(),
            status: "completed".to_string(),
            language: "Rust".to_string(),
            effort: Some("low".to_string()),
            started_at: "2026-09-04T12:00:00Z".to_string(),
            completed_at: "2026-09-04T12:01:00Z".to_string(),
            duration_seconds: 60.0,
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            stages: vec![],
            throughput_req_sec: Some(40000.0),
            tokens: RunTokenUsage { prompt_tokens: 500, cached_tokens: 0, completion_tokens: 100, total_tokens: 600 },
            cost_usd: 0.005,
            savings_percent: 0.0,
            efficiency_score: 200.0,
            files: vec![],
            env: None,
            git_commit: None,
            is_published: None,
            method_version: Some("0.1.0".to_string()),
        };

        fs::write(run_dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

        let pub_res = RunPublisher::publish_run(
            &temp_dir,
            &runs_dir,
            &results_dir,
            run_id,
            Some("bench: published test run 42"),
        );

        assert!(pub_res.is_ok(), "Publish run failed: {:?}", pub_res.err());
        let res = pub_res.unwrap();
        assert!(res.success);
        assert_eq!(res.run_id, run_id);
        assert!(temp_dir.join("SUMMARY.md").exists());
        assert!(run_dir.join("env.json").exists());
        assert!(run_dir.join("README.md").exists());

        // Verify git status in temp repo - runs and results must be committed, not untracked or ignored
        let status_out = Command::new("git").current_dir(&temp_dir).args(["status", "--porcelain"]).output().unwrap();
        let status_str = String::from_utf8_lossy(&status_out.stdout);
        assert_eq!(status_str.trim(), "", "Working tree should be clean after publish");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
