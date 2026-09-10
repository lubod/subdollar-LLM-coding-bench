use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use colored::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use subdollar_bench::config::{Cli, Commands, TaskType};
use subdollar_bench::pipeline::{BenchmarkConfig, BenchmarkPipeline, PipelineLogger};
use subdollar_bench::report::{LeaderboardManager, RunPublisher, SummaryGenerator};
use subdollar_bench::verifier::{DnsVerifier, HttpVerifier, RedisVerifier};
use subdollar_bench::web;
use subdollar_bench::web::server::compute_pass_at_k;

struct CliPipelineLogger;

impl PipelineLogger for CliPipelineLogger {
    fn log(&self, msg: &str) {
        println!("{}", msg);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let cli = Cli::parse();
    run_cli(cli).await
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Run {
            model,
            task,
            effort,
            max_turns,
            budget_usd,
            timeout_min,
            api_key,
            workdir,
            eval_only,
            trials,
            no_save,
        } => {
            let total_trials = trials.max(1);
            let mut passing_trials = 0;
            let logger = Arc::new(CliPipelineLogger);

            let work_path_buf = if Path::new(&workdir).is_absolute() {
                PathBuf::from(&workdir)
            } else {
                subdollar_bench::config::get_repo_root().join(&workdir)
            };

            for trial_idx in 1..=total_trials {
                if total_trials > 1 {
                    println!(
                        "{}",
                        format!("=== Starting Trial {}/{} ===", trial_idx, total_trials)
                            .bold()
                            .blue()
                    );
                }

                let run_id = if total_trials > 1 {
                    format!(
                        "{}_{}_trial{}_{}",
                        task,
                        model.replace(['/', ':'], "_"),
                        trial_idx,
                        Utc::now().format("%Y%m%d_%H%M%S")
                    )
                } else {
                    format!(
                        "{}_{}_{}",
                        task,
                        model.replace(['/', ':'], "_"),
                        Utc::now().format("%Y%m%d_%H%M%S")
                    )
                };

                let pipe_config = BenchmarkConfig {
                    model: model.clone(),
                    task,
                    effort: effort.clone(),
                    max_turns,
                    budget_usd,
                    timeout_min,
                    api_key: api_key.clone(),
                    workdir: work_path_buf.clone(),
                    eval_only,
                    run_id: Some(run_id),
                    save_results: !no_save,
                };

                let pipe_res = BenchmarkPipeline::execute(pipe_config, logger.clone(), None).await;

                match pipe_res {
                    Ok(output) => {
                        if output.benchmark_result.pass_rate == 100.0 {
                            passing_trials += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("  [ERROR] Trial execution failed: {}", e);
                    }
                }
            }

            if total_trials > 1 {
                let pass_at_1 = compute_pass_at_k(total_trials as usize, passing_trials, 1) * 100.0;
                let pass_at_k =
                    compute_pass_at_k(total_trials as usize, passing_trials, total_trials as usize)
                        * 100.0;
                println!(
                    "\n{}",
                    "=== Multi-Trial Pass@k Summary ===".bold().magenta()
                );
                println!("  Total Trials:   {}", total_trials);
                println!("  Passing Trials: {}", passing_trials);
                println!("  Pass@1:         {:.1}%", pass_at_1);
                println!("  Pass@{}:         {:.1}%", total_trials, pass_at_k);
                println!("{}", "=================================".bold().magenta());
            }

            let results_dir_buf = subdollar_bench::config::get_repo_root().join("results");
            let results_dir_str = results_dir_buf.to_str().unwrap_or("./results");
            let all = LeaderboardManager::load_all(results_dir_str);
            LeaderboardManager::print_table(&all);
        }

        Commands::Eval { task, port } => {
            let p = port.unwrap_or(match task {
                TaskType::Redis => 6379,
                TaskType::Http => 8080,
                TaskType::Dns => 5353,
            });
            println!("Evaluating {} on port {}...", task, p);
            match task {
                TaskType::Redis => {
                    let v = RedisVerifier::new(p, None);
                    let res = v.run_all().await;
                    println!(
                        "Pass rate: {:.1}% ({}/{})",
                        res.pass_rate, res.passed_count, res.total_stages
                    );
                }
                TaskType::Http => {
                    let v = HttpVerifier::new(p, None);
                    let res = v.run_all().await;
                    println!(
                        "Pass rate: {:.1}% ({}/{})",
                        res.pass_rate, res.passed_count, res.total_stages
                    );
                }
                TaskType::Dns => {
                    let v = DnsVerifier::new(p, None);
                    let res = v.run_all().await;
                    println!(
                        "Pass rate: {:.1}% ({}/{})",
                        res.pass_rate, res.passed_count, res.total_stages
                    );
                }
            }
        }

        Commands::Leaderboard { results_dir, task } => {
            let all = LeaderboardManager::load_all(&results_dir);
            if all.is_empty() {
                println!("No benchmark results found in {}", results_dir);
            } else {
                LeaderboardManager::print_leaderboard_views(&all, &task);
            }
        }

        Commands::Publish {
            run_id,
            message,
            runs_dir,
            results_dir,
            repo_root,
        } => {
            println!(
                "{}",
                format!("Publishing run {} to Git...", run_id).bold().cyan()
            );
            let res = RunPublisher::publish_run(
                Path::new(&repo_root),
                Path::new(&runs_dir),
                Path::new(&results_dir),
                &run_id,
                message.as_deref(),
            )?;
            println!("{}", "✅ Successfully published run to Git!".bold().green());
            println!("  Commit:  {}", res.commit_hash.yellow());
            println!("  Message: {}", res.commit_message);
            println!("  Summary: {}", res.summary_path);
            println!("  Files committed:");
            for f in &res.files_committed {
                println!("   - {}", f);
            }
        }

        Commands::Summary {
            runs_dir,
            repo_root,
        } => {
            println!(
                "{}",
                "Regenerating SUMMARY.md from recorded runs..."
                    .bold()
                    .cyan()
            );
            let summary_path =
                SummaryGenerator::update_summary_file(Path::new(&repo_root), Path::new(&runs_dir))?;
            println!(
                "✅ Successfully updated {}",
                summary_path.display().to_string().bold().green()
            );
        }

        Commands::Ui { port, host } => {
            web::UiServer::start(&host, port).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use subdollar_bench::report::{BenchmarkRunResult, RunArchiver, RunManifest, RunTokenUsage};

    #[tokio::test]
    async fn test_main_cli_leaderboard() {
        let temp_dir = std::env::temp_dir().join(format!("test_main_lb_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let cli_empty = Cli {
            command: Commands::Leaderboard {
                results_dir: temp_dir.to_string_lossy().to_string(),
                task: "all".to_string(),
            },
        };
        assert!(run_cli(cli_empty).await.is_ok());

        let result = BenchmarkRunResult {
            id: "test_run_main_1".to_string(),
            model: "test_model".to_string(),
            task: "redis".to_string(),
            language: "Rust".to_string(),
            effort: Some("low".to_string()),
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            throughput_req_sec: Some(1000.0),
            reference_throughput_req_sec: Some(71000.0),
            prompt_tokens: 100,
            cached_tokens: 50,
            completion_tokens: 20,
            total_cost_usd: 0.01,
            effective_cost_usd: Some(0.015),
            duration_seconds: Some(60.0),
            turns: Some(4),
            savings_percent: 10.0,
            efficiency_score: 100.0,
            throughput_score: Some(102.8),
            timestamp: "2026-09-04T12:00:00Z".to_string(),
            method_version: Some("0.1.0".to_string()),
        };
        let _ = LeaderboardManager::save_result(&temp_dir.to_string_lossy(), &result);

        let cli_with_res = Cli {
            command: Commands::Leaderboard {
                results_dir: temp_dir.to_string_lossy().to_string(),
                task: "redis".to_string(),
            },
        };
        assert!(run_cli(cli_with_res).await.is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_cli_summary() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_main_summary_{}", std::process::id()));
        let runs_dir = temp_dir.join("runs");
        let repo_root = temp_dir.join("repo");
        let _ = fs::create_dir_all(&runs_dir);
        let _ = fs::create_dir_all(&repo_root);

        let cli = Cli {
            command: Commands::Summary {
                runs_dir: runs_dir.to_string_lossy().to_string(),
                repo_root: repo_root.to_string_lossy().to_string(),
            },
        };
        assert!(run_cli(cli).await.is_ok());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_cli_eval() {
        let cli_redis = Cli {
            command: Commands::Eval {
                task: TaskType::Redis,
                port: Some(59997),
            },
        };
        assert!(run_cli(cli_redis).await.is_ok());

        let cli_http = Cli {
            command: Commands::Eval {
                task: TaskType::Http,
                port: Some(59998),
            },
        };
        assert!(run_cli(cli_http).await.is_ok());

        let cli_dns = Cli {
            command: Commands::Eval {
                task: TaskType::Dns,
                port: Some(59999),
            },
        };
        assert!(run_cli(cli_dns).await.is_ok());
    }

    #[tokio::test]
    async fn test_main_cli_publish() {
        let temp_dir = std::env::temp_dir().join(format!("test_main_pub_{}", std::process::id()));
        let repo_root = temp_dir.join("repo");
        let runs_dir = repo_root.join("runs");
        let results_dir = repo_root.join("results");
        let _ = fs::create_dir_all(&repo_root);
        let _ = fs::create_dir_all(&runs_dir);
        let _ = fs::create_dir_all(&results_dir);

        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "Bench Tester"])
            .current_dir(&repo_root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "tester@bench.local"])
            .current_dir(&repo_root)
            .output();
        fs::write(repo_root.join("README.md"), "# Init").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&repo_root)
            .output();

        let run_id = "test_main_publish_run";
        let run_dir = runs_dir.join(run_id);
        let _ = fs::create_dir_all(run_dir.join("workspace"));
        fs::write(run_dir.join("workspace/main.rs"), "fn main() {}").unwrap();

        let manifest = RunManifest {
            run_id: run_id.to_string(),
            model: "test_model".to_string(),
            task: "redis".to_string(),
            status: "completed".to_string(),
            language: "Rust".to_string(),
            effort: Some("low".to_string()),
            started_at: "2026-09-04T12:00:00Z".to_string(),
            completed_at: "2026-09-04T12:01:00Z".to_string(),
            duration_seconds: 60.0,
            turns: Some(3),
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            stages: Vec::new(),
            throughput_req_sec: Some(1000.0),
            reference_throughput_req_sec: Some(71000.0),
            tokens: RunTokenUsage {
                prompt_tokens: 100,
                cached_tokens: 50,
                completion_tokens: 20,
                total_tokens: 120,
            },
            cost_usd: 0.01,
            effective_cost_usd: Some(0.015),
            savings_percent: 10.0,
            efficiency_score: 100.0,
            throughput_score: Some(102.8),
            files: Vec::new(),
            env: None,
            git_commit: None,
            is_published: None,
            method_version: Some("0.1.0".to_string()),
        };
        fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let cli = Cli {
            command: Commands::Publish {
                run_id: run_id.to_string(),
                message: Some("Main test publish".to_string()),
                runs_dir: runs_dir.to_string_lossy().to_string(),
                results_dir: results_dir.to_string_lossy().to_string(),
                repo_root: repo_root.to_string_lossy().to_string(),
            },
        };
        assert!(run_cli(cli).await.is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_cli_run_eval_only_redis() {
        let temp_workdir =
            std::env::temp_dir().join(format!("test_main_run_redis_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_workdir);

        let cli_run = Cli {
            command: Commands::Run {
                model: "google/gemini-2.5-flash".to_string(),
                task: TaskType::Redis,
                effort: "low".to_string(),
                max_turns: 1,
                budget_usd: 0.10,
                timeout_min: 1,
                api_key: None,
                workdir: temp_workdir.to_string_lossy().to_string(),
                eval_only: true,
                trials: 2,
                no_save: true,
            },
        };
        assert!(run_cli(cli_run).await.is_ok());

        let _ = fs::remove_dir_all(&temp_workdir);

        let runs_dir = RunArchiver::resolve_runs_dir();
        if let Ok(entries) = fs::read_dir(&runs_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        if let Ok(entries) = fs::read_dir("./results") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    #[tokio::test]
    async fn test_main_cli_run_eval_only_http_with_candidate() {
        let temp_workdir =
            std::env::temp_dir().join(format!("test_main_run_http_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_workdir);
        let start_sh = temp_workdir.join("start.sh");
        fs::write(&start_sh, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&start_sh, fs::Permissions::from_mode(0o755));
        }

        let cli_run = Cli {
            command: Commands::Run {
                model: "google/gemini-2.5-flash".to_string(),
                task: TaskType::Http,
                effort: "low".to_string(),
                max_turns: 1,
                budget_usd: 0.10,
                timeout_min: 1,
                api_key: None,
                workdir: temp_workdir.to_string_lossy().to_string(),
                eval_only: true,
                trials: 1,
                no_save: true,
            },
        };
        assert!(run_cli(cli_run).await.is_ok());

        let _ = fs::remove_dir_all(&temp_workdir);

        let runs_dir = RunArchiver::resolve_runs_dir();
        if let Ok(entries) = fs::read_dir(&runs_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        let res_dir = subdollar_bench::config::get_repo_root().join("results");
        if let Ok(entries) = fs::read_dir(&res_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    #[tokio::test]
    async fn test_main_cli_compliance_disqualification() {
        let temp_workdir =
            std::env::temp_dir().join(format!("test_main_run_cheat_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_workdir);
        let start_sh = temp_workdir.join("start.sh");
        fs::write(&start_sh, "#!/bin/sh\nredis-server --port 6379\n").unwrap();

        let cli_run = Cli {
            command: Commands::Run {
                model: "google/gemini-2.5-flash".to_string(),
                task: TaskType::Redis,
                effort: "low".to_string(),
                max_turns: 1,
                budget_usd: 0.10,
                timeout_min: 1,
                api_key: None,
                workdir: temp_workdir.to_string_lossy().to_string(),
                eval_only: true,
                trials: 1,
                no_save: true,
            },
        };
        assert!(run_cli(cli_run).await.is_ok());

        let _ = fs::remove_dir_all(&temp_workdir);

        let runs_dir = RunArchiver::resolve_runs_dir();
        if let Ok(entries) = fs::read_dir(&runs_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        let res_dir = subdollar_bench::config::get_repo_root().join("results");
        if let Ok(entries) = fs::read_dir(&res_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    #[tokio::test]
    async fn test_main_cli_ui_cancelled() {
        let cli_ui = Cli {
            command: Commands::Ui {
                host: "127.0.0.1".to_string(),
                port: 0,
            },
        };
        let _ = tokio::time::timeout(Duration::from_millis(150), run_cli(cli_ui)).await;
    }
}
