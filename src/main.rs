use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::Parser;
use colored::*;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

use subdollar_bench::bench;
use subdollar_bench::config::{Cli, Commands, TaskType};
use subdollar_bench::cost::ModelPricing;
use subdollar_bench::report::{
    BenchmarkRunResult, LeaderboardManager, RunArchiver, RunManifest, RunPublisher, RunTokenUsage,
    SummaryGenerator,
};
use subdollar_bench::sandbox::{AgentExecutionLimits, OmpRunner, SandboxManager};
use subdollar_bench::verifier::{HttpVerifier, RedisVerifier};
use subdollar_bench::web;

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
        } => {
            let start_instant = std::time::Instant::now();
            let started_at = Utc::now().to_rfc3339();

            println!("{}", "=========================================================".bold().blue());
            println!("  {} - Autonomous Under-$1 LLM Coding Benchmark", "SubDollarBench".bold().cyan());
            println!("  Model:   {}", model.bold().yellow());
            println!("  Effort:  {}", effort.bold().cyan());
            println!("  Task:    {}", task.to_string().bold().green());
            println!("  Budget:  ${:.2}", budget_usd);
            println!("  Turns:   {}", max_turns);
            println!("  Timeout: {}m", timeout_min);
            println!("{}", "=========================================================".bold().blue());

            let work_path = Path::new(&workdir);
            if !eval_only {
                let _ = fs::remove_dir_all(work_path);
            }
            fs::create_dir_all(work_path)?;

            let sandbox = SandboxManager::new();

            // Initial OpenRouter spend checkpoint
            let initial_spend = if let Some(key) = &api_key {
                ModelPricing::query_openrouter_key_usage(key).await
            } else {
                None
            };
            if let Some(init) = initial_spend {
                println!("  [Account] Initial OpenRouter key spend: ${:.4} USD", init);
            }

            // 1. Start reference services in Docker
            match task {
                TaskType::Redis => {
                    let _ = sandbox.start_reference_redis(6380);
                }
                TaskType::Http => {
                    let _ = sandbox.start_reference_http(8081);
                }
            }

            // 2. Read task prompt
            let prompt_file = format!("tasks/{}/prompt.md", task);
            let prompt_content = fs::read_to_string(&prompt_file)
                .map_err(|_| anyhow!("Could not read prompt file: {}", prompt_file))?;

            // 3. Run OMP Agent (unless eval_only)
            let (omp_stats, prompt_tokens, cached_tokens, completion_tokens) = if !eval_only {
                println!("\n{}", ">>> Spawning OMP Agent in headless mode...".bold().magenta());
                let limits = AgentExecutionLimits {
                    max_turns: if max_turns > 0 { Some(max_turns) } else { None },
                    max_budget_usd: if budget_usd > 0.0 { Some(budget_usd) } else { None },
                    timeout_seconds: if timeout_min > 0 { Some(timeout_min * 60) } else { None },
                };
                let stats = OmpRunner::run_agent(&model, &prompt_content, work_path, api_key.as_deref(), limits, Some(&effort))?;
                let p = stats.prompt_tokens;
                let c = stats.cached_tokens;
                let comp = stats.completion_tokens;
                (stats, p, c, comp)
            } else {
                println!("\n{}", ">>> Skipping OMP agent run (--eval-only set)".italic());
                (Default::default(), 0, 0, 0)
            };

            // 4. Start candidate server inside Docker
            let target_port = match task {
                TaskType::Redis => 6379,
                TaskType::Http => 8080,
            };

            let has_runnable = sandbox.ensure_runnable_candidate(work_path).unwrap_or(false);
            if !has_runnable {
                println!("{}", ">>> [ERROR] No runnable start.sh or Dockerfile found in ./workspace!".bold().red());
            } else {
                println!("{}", ">>> Starting candidate container inside isolated Docker sandbox...".bold().cyan());
                let _ = sandbox.start_candidate_in_docker(work_path, target_port);
                println!("{}", format!(">>> Waiting for candidate port {} readiness (timeout: 30s)...", target_port).cyan());
                let ready = sandbox.wait_for_port(target_port, 30).await;
                if ready {
                    println!("{}", format!(">>> Candidate server online on port {}!", target_port).green());
                } else {
                    println!("{}", format!(">>> [WARN] Candidate port {} not responding after 30s. Proceeding to tests...", target_port).yellow());
                }
            }

            // 5. Run Verification
            println!("\n{}", ">>> Running Protocol Verification Test Suite...".bold().cyan());
            let (pass_rate, passed_stages, total_stages, stages) = match task {
                TaskType::Redis => {
                    let verifier = RedisVerifier::new(6379, Some(6380));
                    let summary = verifier.run_all().await;
                    for stage in &summary.stages {
                        if stage.passed {
                            println!("  [PASS] {}", stage.name.green());
                        } else {
                            println!("  [FAIL] {}", stage.name.red());
                            if let Some(err) = &stage.error {
                                println!("         Error: {}", err.dimmed());
                            }
                        }
                    }
                    (summary.pass_rate, summary.passed_count, summary.total_stages, summary.stages)
                }
                TaskType::Http => {
                    let verifier = HttpVerifier::new(8080);
                    let summary = verifier.run_all().await;
                    for stage in &summary.stages {
                        if stage.passed {
                            println!("  [PASS] {}", stage.name.green());
                        } else {
                            println!("  [FAIL] {}", stage.name.red());
                            if let Some(err) = &stage.error {
                                println!("         Error: {}", err.dimmed());
                            }
                        }
                    }
                    (summary.pass_rate, summary.passed_count, summary.total_stages, summary.stages)
                }
            };

            // 6. Concurrency / Load benchmark
            let mut throughput = None;
            if pass_rate >= 75.0 {
                println!("\n{}", ">>> Running Stress & Concurrency Benchmark in Docker...".bold().cyan());
                match task {
                    TaskType::Redis => {
                        match bench::BenchmarkRunner::run_redis_benchmark(6379) {
                            Ok(tp) => {
                                println!("  Throughput: {} req/sec", format!("{:.0}", tp).bold().green());
                                throughput = Some(tp);
                            }
                            Err(e) => warn!("Redis benchmark failed: {}", e),
                        }
                    }
                    TaskType::Http => {
                        match bench::BenchmarkRunner::run_wrk_benchmark(8080) {
                            Ok(tp) => {
                                println!("  Throughput: {} req/sec", format!("{:.0}", tp).bold().green());
                                throughput = Some(tp);
                            }
                            Err(e) => warn!("HTTP benchmark failed: {}", e),
                        }
                    }
                }
            }

            // 7. Cost & Token calculation (Cache-Aware + Live OpenRouter Delta)
            let pricing = ModelPricing::for_model(&model);
            let breakdown = pricing.compute_cost_with_cache(prompt_tokens, cached_tokens, completion_tokens);

            let mut live_spend_delta = None;
            if let (Some(key), Some(init)) = (&api_key, initial_spend) {
                sleep(Duration::from_millis(1500)).await;
                if let Some(fin) = ModelPricing::query_openrouter_key_usage(key).await {
                    if fin >= init {
                        live_spend_delta = Some(fin - init);
                    }
                }
            }

            let cost_usd = live_spend_delta.unwrap_or(breakdown.total_cost_usd);
            let cost_cents = (cost_usd * 100.0).max(0.01);
            let efficiency_score = pass_rate / cost_cents;
            let lang = LeaderboardManager::detect_language(work_path);

            let completed_at = Utc::now().to_rfc3339();
            let duration_seconds = start_instant.elapsed().as_secs_f64();
            let run_id = format!("{}_{}_{}", task, model.replace('/', "_").replace(':', "_"), Utc::now().format("%Y%m%d_%H%M%S"));

            let scanned_files = RunArchiver::scan_workspace_files(work_path);
            let manifest = RunManifest {
                run_id: run_id.clone(),
                model: model.clone(),
                task: task.to_string(),
                status: if pass_rate == 100.0 { "completed".to_string() } else { "failed_tests".to_string() },
                language: lang.clone(),
                effort: Some(effort.clone()),
                started_at,
                completed_at: completed_at.clone(),
                duration_seconds,
                pass_rate,
                passed_stages,
                total_stages,
                stages,
                throughput_req_sec: throughput,
                tokens: RunTokenUsage {
                    prompt_tokens,
                    cached_tokens,
                    completion_tokens,
                    total_tokens: omp_stats.total_tokens.max(prompt_tokens + completion_tokens),
                },
                cost_usd,
                savings_percent: breakdown.savings_percent,
                efficiency_score,
                files: scanned_files,
                env: None,
                git_commit: None,
                is_published: None,
            };

            let runs_dir = RunArchiver::resolve_runs_dir();
            let cli_log = format!("Run {} (Effort: {}) completed with pass_rate {:.1}%", run_id, effort, pass_rate);
            let _ = RunArchiver::archive_run(&runs_dir, &manifest, work_path, &cli_log);

            let result = BenchmarkRunResult {
                id: run_id,
                model: model.clone(),
                task: task.to_string(),
                language: lang,
                effort: Some(effort),
                pass_rate,
                passed_stages,
                total_stages,
                throughput_req_sec: throughput,
                prompt_tokens,
                cached_tokens,
                completion_tokens,
                total_cost_usd: cost_usd,
                savings_percent: breakdown.savings_percent,
                efficiency_score,
                timestamp: completed_at,
            };

            LeaderboardManager::save_result("./results", &result)?;

            println!("\n{}", "=========================================================".bold().blue());
            println!("  Results Summary for {}:", model.bold());
            println!("  - Language Chosen: {}", result.language.bold().cyan());
            println!("  - Effort Level:    {}", result.effort.as_deref().unwrap_or("auto").bold().yellow());
            println!("  - Pass Rate:       {:.1}% ({}/{})", pass_rate, passed_stages, total_stages);
            println!("  - Prompt Tokens:   {} (Cached: {})", prompt_tokens, cached_tokens);
            println!("  - Cache Savings:   {:.1}% (${:.4} saved)", breakdown.savings_percent, breakdown.savings_usd);
            if let Some(delta) = live_spend_delta {
                println!("  - OpenRouter Live: ${:.4} USD (verified directly)", delta);
            }
            println!("  - Final Cost:      ${:.4} USD", cost_usd);
            println!("  - Efficiency:      {:.1} points / cent", efficiency_score);
            println!("  - Archived to:     runs/{}/", result.id.bold());
            println!("{}", "=========================================================".bold().blue());

            let all = LeaderboardManager::load_all("./results");
            LeaderboardManager::print_table(&all);

            sandbox.cleanup();
        }

        Commands::Eval { task, port } => {
            let p = port.unwrap_or(match task {
                TaskType::Redis => 6379,
                TaskType::Http => 8080,
            });
            println!("Evaluating {} on port {}...", task, p);
            match task {
                TaskType::Redis => {
                    let v = RedisVerifier::new(p, None);
                    let res = v.run_all().await;
                    println!("Pass rate: {:.1}% ({}/{})", res.pass_rate, res.passed_count, res.total_stages);
                }
                TaskType::Http => {
                    let v = HttpVerifier::new(p);
                    let res = v.run_all().await;
                    println!("Pass rate: {:.1}% ({}/{})", res.pass_rate, res.passed_count, res.total_stages);
                }
            }
        }

        Commands::Leaderboard { results_dir } => {
            let all = LeaderboardManager::load_all(&results_dir);
            if all.is_empty() {
                println!("No benchmark results found in {}", results_dir);
            } else {
                LeaderboardManager::print_table(&all);
            }
        }

        Commands::Publish {
            run_id,
            message,
            runs_dir,
            results_dir,
            repo_root,
        } => {
            println!("{}", format!("Publishing run {} to Git...", run_id).bold().cyan());
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

        Commands::Summary { runs_dir, repo_root } => {
            println!("{}", "Regenerating SUMMARY.md from recorded runs...".bold().cyan());
            let summary_path = SummaryGenerator::update_summary_file(
                Path::new(&repo_root),
                Path::new(&runs_dir),
            )?;
            println!("✅ Successfully updated {}", summary_path.display().to_string().bold().green());
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

    #[tokio::test]
    async fn test_main_cli_leaderboard() {
        let temp_dir = std::env::temp_dir().join(format!("test_main_lb_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Empty leaderboard
        let cli_empty = Cli {
            command: Commands::Leaderboard {
                results_dir: temp_dir.to_string_lossy().to_string(),
            },
        };
        assert!(run_cli(cli_empty).await.is_ok());

        // 2. Leaderboard with a result
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
            prompt_tokens: 100,
            cached_tokens: 50,
            completion_tokens: 20,
            total_cost_usd: 0.01,
            savings_percent: 10.0,
            efficiency_score: 100.0,
            timestamp: "2026-09-04T12:00:00Z".to_string(),
        };
        let _ = LeaderboardManager::save_result(&temp_dir.to_string_lossy(), &result);

        let cli_with_res = Cli {
            command: Commands::Leaderboard {
                results_dir: temp_dir.to_string_lossy().to_string(),
            },
        };
        assert!(run_cli(cli_with_res).await.is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_cli_summary() {
        let temp_dir = std::env::temp_dir().join(format!("test_main_summary_{}", std::process::id()));
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
        // Run eval on unused ports
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
    }

    #[tokio::test]
    async fn test_main_cli_publish() {
        let temp_dir = std::env::temp_dir().join(format!("test_main_pub_{}", std::process::id()));
        let repo_root = temp_dir.join("repo");
        let runs_dir = temp_dir.join("runs");
        let results_dir = temp_dir.join("results");
        let _ = fs::create_dir_all(&repo_root);
        let _ = fs::create_dir_all(&runs_dir);
        let _ = fs::create_dir_all(&results_dir);

        // Init git repo
        let _ = std::process::Command::new("git").args(["init"]).current_dir(&repo_root).output();
        let _ = std::process::Command::new("git").args(["config", "user.name", "Bench Tester"]).current_dir(&repo_root).output();
        let _ = std::process::Command::new("git").args(["config", "user.email", "tester@bench.local"]).current_dir(&repo_root).output();
        fs::write(repo_root.join("README.md"), "# Init").unwrap();
        let _ = std::process::Command::new("git").args(["add", "."]).current_dir(&repo_root).output();
        let _ = std::process::Command::new("git").args(["commit", "-m", "Initial commit"]).current_dir(&repo_root).output();

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
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            stages: Vec::new(),
            throughput_req_sec: Some(1000.0),
            tokens: RunTokenUsage {
                prompt_tokens: 100,
                cached_tokens: 50,
                completion_tokens: 20,
                total_tokens: 120,
            },
            cost_usd: 0.01,
            savings_percent: 10.0,
            efficiency_score: 100.0,
            files: Vec::new(),
            env: None,
            git_commit: None,
            is_published: None,
        };
        fs::write(run_dir.join("manifest.json"), serde_json::to_string(&manifest).unwrap()).unwrap();

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
        let temp_workdir = std::env::temp_dir().join(format!("test_main_run_redis_{}", std::process::id()));
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
            },
        };
        assert!(run_cli(cli_run).await.is_ok());

        let _ = fs::remove_dir_all(&temp_workdir);

        // Clean any gemini/test artifacts from runs/ and results/
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
        let temp_workdir = std::env::temp_dir().join(format!("test_main_run_http_{}", std::process::id()));
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
            },
        };
        assert!(run_cli(cli_run).await.is_ok());

        let _ = fs::remove_dir_all(&temp_workdir);

        // Clean any gemini/test artifacts from runs/ and results/
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
    async fn test_main_cli_ui_cancelled() {
        let cli_ui = Cli {
            command: Commands::Ui {
                host: "127.0.0.1".to_string(),
                port: 0,
            },
        };
        // Run and cancel after a short duration
        let _ = tokio::time::timeout(Duration::from_millis(150), run_cli(cli_ui)).await;
    }
}
