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
use subdollar_bench::sandbox::{OmpRunner, SandboxManager};
use subdollar_bench::verifier::{HttpVerifier, RedisVerifier};
use subdollar_bench::web;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            model,
            task,
            effort,
            max_turns,
            budget_usd,
            api_key,
            workdir,
            eval_only,
        } => {
            let start_instant = std::time::Instant::now();
            let started_at = Utc::now().to_rfc3339();

            println!("{}", "=========================================================".bold().blue());
            println!("  {} - Autonomous Under-$1 LLM Coding Benchmark", "SubDollarBench".bold().cyan());
            println!("  Model:  {}", model.bold().yellow());
            println!("  Effort: {}", effort.bold().cyan());
            println!("  Task:   {}", task.to_string().bold().green());
            println!("  Budget: ${:.2}", budget_usd);
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
                let stats = OmpRunner::run_agent(&model, &prompt_content, work_path, api_key.as_deref(), max_turns, Some(&effort))?;
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
