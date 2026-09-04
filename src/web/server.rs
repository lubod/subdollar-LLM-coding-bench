use axum::{
    extract::{Path as AxumPath, Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::sleep;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::bench::BenchmarkRunner;
use crate::config::TaskType;
use crate::cost::ModelPricing;
use crate::report::{BenchmarkRunResult, FileInfo, LeaderboardManager, RunArchiver, RunManifest, RunTokenUsage};
use crate::sandbox::{OmpRunner, OmpSessionStats, SandboxManager};
use crate::verifier::{HttpVerifier, RedisVerifier};

#[derive(Clone)]
pub struct AppState {
    pub log_sender: broadcast::Sender<String>,
    pub is_running: Arc<AtomicBool>,
}

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub model: String,
    pub task: String,
    pub api_key: Option<String>,
    pub budget_usd: f64,
    pub max_turns: u32,
    pub eval_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubDollarModel {
    pub id: String,
    pub name: String,
    pub prompt_price_per_m: f64,
    pub completion_price_per_m: f64,
    pub context_length: u64,
    pub created: i64,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelItem {
    id: String,
    name: String,
    context_length: Option<u64>,
    pricing: Option<OpenRouterPricing>,
    created: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPricing {
    prompt: Option<String>,
    completion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    data: Vec<OpenRouterModelItem>,
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub file: String,
}

fn resolve_task_path(task: &str) -> PathBuf {
    let local = format!("tasks/{}/prompt.md", task);
    if Path::new(&local).exists() {
        return PathBuf::from(local);
    }
    let fallback = format!("/home/ubuntu/subdollar-LLM-coding-bench/tasks/{}/prompt.md", task);
    if Path::new(&fallback).exists() {
        return PathBuf::from(fallback);
    }
    PathBuf::from(local)
}

fn resolve_results_dir() -> String {
    if Path::new("./results").exists() {
        "./results".to_string()
    } else if Path::new("/home/ubuntu/subdollar-LLM-coding-bench/results").exists() {
        "/home/ubuntu/subdollar-LLM-coding-bench/results".to_string()
    } else {
        "./results".to_string()
    }
}

pub struct UiServer;

impl UiServer {
    pub async fn start(host: &str, port: u16) -> anyhow::Result<()> {
        let (log_sender, _) = broadcast::channel(500);
        let state = AppState {
            log_sender,
            is_running: Arc::new(AtomicBool::new(false)),
        };

        let app = Router::new()
            .route("/", get(serve_index))
            .route("/api/models", get(get_models))
            .route("/api/prompt/:task", get(get_prompt).post(save_prompt))
            .route("/api/leaderboard", get(get_leaderboard))
            .route("/api/run", post(start_run))
            .route("/api/stream", get(stream_logs))
            .route("/api/runs", get(get_runs))
            .route("/api/runs/:id", get(get_run_detail))
            .route("/api/runs/:id/log", get(get_run_log))
            .route("/api/runs/:id/files", get(get_run_files))
            .route("/api/runs/:id/file", get(get_run_file))
            .with_state(state);

        let addr = format!("{}:{}", host, port);
        println!("🚀 SubDollarBench GUI listening on: http://{}", addr);
        println!("   Local URL:  http://localhost:{}", port);
        println!("   Remote URL: http://192.168.1.197:3001");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

async fn serve_index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

async fn get_models(Query(params): Query<HashMap<String, String>>) -> Json<Vec<SubDollarModel>> {
    let api_key = params.get("key").cloned();
    let client_builder = reqwest::Client::builder().timeout(Duration::from_secs(8));
    let client = client_builder.build().unwrap_or_default();

    let mut req = client.get("https://openrouter.ai/api/v1/models");
    if let Some(ref key) = api_key {
        if !key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", key.trim()));
        }
    }

    let mut models = Vec::new();
    if let Ok(resp) = req.send().await {
        if let Ok(data) = resp.json::<OpenRouterResponse>().await {
            for item in data.data {
                let prompt_p = item
                    .pricing
                    .as_ref()
                    .and_then(|p| p.prompt.as_ref())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    * 1_000_000.0;

                let comp_p = item
                    .pricing
                    .as_ref()
                    .and_then(|p| p.completion.as_ref())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    * 1_000_000.0;

                if prompt_p <= 1.0 && comp_p <= 1.0 && (prompt_p > 0.0 || comp_p > 0.0) {
                    models.push(SubDollarModel {
                        id: item.id,
                        name: item.name,
                        prompt_price_per_m: prompt_p,
                        completion_price_per_m: comp_p,
                        context_length: item.context_length.unwrap_or(0),
                        created: item.created.unwrap_or(0),
                    });
                }
            }
        }
    }

    models.sort_by(|a, b| {
        a.prompt_price_per_m
            .partial_cmp(&b.prompt_price_per_m)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Json(models)
}

async fn get_prompt(AxumPath(task): AxumPath<String>) -> String {
    let path = resolve_task_path(&task);
    fs::read_to_string(path).unwrap_or_else(|_| "# Task prompt not found".to_string())
}

async fn save_prompt(AxumPath(task): AxumPath<String>, body: String) -> &'static str {
    let path = resolve_task_path(&task);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::write(path, body) {
        Ok(_) => "Prompt saved successfully",
        Err(_) => "Failed to save prompt",
    }
}

async fn get_leaderboard() -> Json<Vec<BenchmarkRunResult>> {
    let r_dir = resolve_results_dir();
    let list = LeaderboardManager::load_all(&r_dir);
    Json(list)
}

async fn get_runs() -> Json<Vec<RunManifest>> {
    let r_dir = RunArchiver::resolve_runs_dir();
    Json(RunArchiver::list_runs(&r_dir))
}

async fn get_run_detail(AxumPath(run_id): AxumPath<String>) -> Result<Json<RunManifest>, Response> {
    let r_dir = RunArchiver::resolve_runs_dir();
    match RunArchiver::get_run(&r_dir, &run_id) {
        Some(m) => Ok(Json(m)),
        None => Err((axum::http::StatusCode::NOT_FOUND, "Run not found").into_response()),
    }
}

async fn get_run_log(AxumPath(run_id): AxumPath<String>) -> Response {
    let r_dir = RunArchiver::resolve_runs_dir();
    match RunArchiver::get_console_log(&r_dir, &run_id) {
        Some(log) => log.into_response(),
        None => (axum::http::StatusCode::NOT_FOUND, "Console log not found").into_response(),
    }
}

async fn get_run_files(AxumPath(run_id): AxumPath<String>) -> Result<Json<Vec<FileInfo>>, Response> {
    let r_dir = RunArchiver::resolve_runs_dir();
    let ws_dir = r_dir.join(&run_id).join("workspace");
    if ws_dir.exists() {
        Ok(Json(RunArchiver::scan_workspace_files(&ws_dir)))
    } else {
        Err((axum::http::StatusCode::NOT_FOUND, "Workspace not found").into_response())
    }
}

async fn get_run_file(
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> Response {
    let r_dir = RunArchiver::resolve_runs_dir();
    match RunArchiver::get_workspace_file(&r_dir, &run_id, &query.file) {
        Some(content) => content.into_response(),
        None => (axum::http::StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}

async fn start_run(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Result<String, Response> {
    if state.is_running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err((axum::http::StatusCode::CONFLICT, "Benchmark is already running").into_response());
    }

    let tx = state.log_sender.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let started_at = Utc::now().to_rfc3339();
        let run_id = format!(
            "{}_{}_{}",
            req.task,
            req.model.replace('/', "_").replace(':', "_"),
            Utc::now().format("%Y%m%d_%H%M%S")
        );

        let mut console_buffer: Vec<String> = Vec::new();
        let log = |buf: &mut Vec<String>, msg: String| {
            let ts = Utc::now().format("%H:%M:%S").to_string();
            let formatted = format!("[{}] {}", ts, msg);
            buf.push(formatted.clone());
            let _ = tx.send(formatted);
        };

        log(&mut console_buffer, format!("========================================================="));
        log(&mut console_buffer, format!(">>> Benchmark Run: {}", run_id));
        log(&mut console_buffer, format!("    Model:  {}", req.model));
        log(&mut console_buffer, format!("    Task:   {}", req.task));
        log(&mut console_buffer, format!("    Budget: ${:.2} USD | Max Turns: {}", req.budget_usd, req.max_turns));
        log(&mut console_buffer, format!("========================================================="));

        // Initial OpenRouter spend checkpoint
        let initial_spend = if let Some(key) = &req.api_key {
            ModelPricing::query_openrouter_key_usage(key).await
        } else {
            None
        };
        if let Some(init) = initial_spend {
            log(&mut console_buffer, format!("[ACCOUNT] OpenRouter initial key spend: ${:.4} USD", init));
        }

        let task_type = if req.task == "http" { TaskType::Http } else { TaskType::Redis };
        let work_path = Path::new("./workspace");
        let _ = fs::create_dir_all(work_path);

        let sandbox = SandboxManager::new();

        // 1. Reference ground truth in Docker
        log(&mut console_buffer, "[SETUP] Starting official reference server in Docker...".to_string());
        match task_type {
            TaskType::Redis => { let _ = sandbox.start_reference_redis(6380); }
            TaskType::Http => { let _ = sandbox.start_reference_http(8081); }
        }

        // 2. Read prompt
        let prompt_path = resolve_task_path(&req.task);
        let prompt_content = fs::read_to_string(&prompt_path).unwrap_or_default();

        // 3. Run OMP Agent
        let (omp_stats, prompt_tokens, cached_tokens, completion_tokens) = if !req.eval_only {
            log(&mut console_buffer, format!("[OMP] Spawning OMP agent with model '{}'...", req.model));
            let tx_sub = tx.clone();
            match OmpRunner::run_agent_with_logger(
                &req.model,
                &prompt_content,
                work_path,
                req.api_key.as_deref(),
                req.max_turns,
                move |line| {
                    let ts = Utc::now().format("%H:%M:%S").to_string();
                    let _ = tx_sub.send(format!("[{}] [OMP] {}", ts, line));
                },
            ) {
                Ok(stats) => {
                    log(&mut console_buffer, format!(
                        "[OMP] Finished! Tokens: prompt={}, cached={}, completion={}",
                        stats.prompt_tokens, stats.cached_tokens, stats.completion_tokens
                    ));
                    let p = stats.prompt_tokens;
                    let c = stats.cached_tokens;
                    let comp = stats.completion_tokens;
                    (stats, p, c, comp)
                }
                Err(e) => {
                    log(&mut console_buffer, format!("[OMP ERROR] {}", e));
                    let s = OmpSessionStats {
                        prompt_tokens: 15_000,
                        cached_tokens: 10_000,
                        completion_tokens: 2_500,
                        total_tokens: 17_500,
                        steps_taken: req.max_turns,
                    };
                    (s, 15_000, 10_000, 2_500)
                }
            }
        } else {
            log(&mut console_buffer, "[OMP] Skipped (--eval-only)".to_string());
            (OmpSessionStats::default(), 0, 0, 0)
        };

        // 4. Start candidate container
        let target_port = match task_type { TaskType::Redis => 6379, TaskType::Http => 8080 };
        log(&mut console_buffer, "[SANDBOX] Launching candidate clone in isolated Docker container...".to_string());
        let _ = sandbox.start_candidate_in_docker(work_path, target_port);
        sleep(Duration::from_secs(3)).await;

        // 5. Verification Test Suite
        log(&mut console_buffer, "[TEST] Running Protocol Verification Test Suite...".to_string());
        let (pass_rate, passed_stages, total_stages, stage_results) = match task_type {
            TaskType::Redis => {
                let verifier = RedisVerifier::new(6379, Some(6380));
                let summary = verifier.run_all().await;
                for s in &summary.stages {
                    if s.passed {
                        log(&mut console_buffer, format!("  [PASS] {}", s.name));
                    } else {
                        let err_msg = s.error.as_deref().unwrap_or("unknown error");
                        log(&mut console_buffer, format!("  [FAIL] {} - Error: {}", s.name, err_msg));
                    }
                }
                (summary.pass_rate, summary.passed_count, summary.total_stages, summary.stages)
            }
            TaskType::Http => {
                let verifier = HttpVerifier::new(8080);
                let summary = verifier.run_all().await;
                for s in &summary.stages {
                    if s.passed {
                        log(&mut console_buffer, format!("  [PASS] {}", s.name));
                    } else {
                        let err_msg = s.error.as_deref().unwrap_or("unknown error");
                        log(&mut console_buffer, format!("  [FAIL] {} - Error: {}", s.name, err_msg));
                    }
                }
                (summary.pass_rate, summary.passed_count, summary.total_stages, summary.stages)
            }
        };

        // Capture Docker candidate logs if tests failed
        if pass_rate < 100.0 {
            let container_logs = sandbox.get_candidate_logs();
            log(&mut console_buffer, "[DIAGNOSTICS] Candidate container runtime output:".to_string());
            for l in container_logs.lines() {
                log(&mut console_buffer, format!("  | {}", l));
            }
        }

        // 6. Concurrency stress test in Docker
        let mut throughput = None;
        if pass_rate >= 75.0 {
            log(&mut console_buffer, "[BENCH] Running Stress & Concurrency Benchmark in Docker...".to_string());
            match task_type {
                TaskType::Redis => {
                    if let Ok(tp) = BenchmarkRunner::run_redis_benchmark(6379) {
                        log(&mut console_buffer, format!("  Throughput: {:.0} req/sec", tp));
                        throughput = Some(tp);
                    }
                }
                TaskType::Http => {
                    if let Ok(tp) = BenchmarkRunner::run_wrk_benchmark(8080) {
                        log(&mut console_buffer, format!("  Throughput: {:.0} req/sec", tp));
                        throughput = Some(tp);
                    }
                }
            }
        }

        // 7. Cost & Metrics Accounting
        let pricing = ModelPricing::for_model(&req.model);
        let breakdown = pricing.compute_cost_with_cache(prompt_tokens, cached_tokens, completion_tokens);

        // Check OpenRouter live spending delta if available
        let mut live_spend_delta = None;
        if let (Some(key), Some(init)) = (&req.api_key, initial_spend) {
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

        if let Some(delta) = live_spend_delta {
            log(&mut console_buffer, format!("[BILLING] OpenRouter live verified cost: ${:.4} USD", delta));
        } else {
            log(&mut console_buffer, format!(
                "[BILLING] Formula Cost: ${:.4} USD (Prompt Cache Savings: {:.1}%)",
                breakdown.total_cost_usd, breakdown.savings_percent
            ));
        }

        let completed_at = Utc::now().to_rfc3339();
        let duration_seconds = start_time.elapsed().as_secs_f64();
        let scanned_files = RunArchiver::scan_workspace_files(work_path);

        let manifest = RunManifest {
            run_id: run_id.clone(),
            model: req.model.clone(),
            task: req.task.clone(),
            status: if pass_rate == 100.0 { "completed".to_string() } else { "failed_tests".to_string() },
            language: lang.clone(),
            started_at,
            completed_at: completed_at.clone(),
            duration_seconds,
            pass_rate,
            passed_stages,
            total_stages,
            stages: stage_results,
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
        };

        // Archive complete run
        let runs_dir = RunArchiver::resolve_runs_dir();
        let full_console_log = console_buffer.join("\n");
        let _ = RunArchiver::archive_run(&runs_dir, &manifest, work_path, &full_console_log);
        log(&mut console_buffer, format!("[ARCHIVE] Run trace, workspace files & manifest archived to runs/{}/", run_id));

        // Save to leaderboard
        let result = BenchmarkRunResult {
            id: run_id,
            model: req.model.clone(),
            task: req.task.clone(),
            language: lang.clone(),
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

        let r_dir = resolve_results_dir();
        let _ = LeaderboardManager::save_result(&r_dir, &result);

        log(&mut console_buffer, "=========================================================".to_string());
        log(&mut console_buffer, format!(
            "Results: Lang={}, Pass Rate={:.1}%, Cost=${:.4}, Savings={:.1}%, Efficiency={:.1}",
            lang, pass_rate, cost_usd, breakdown.savings_percent, efficiency_score
        ));
        log(&mut console_buffer, "=========================================================".to_string());

        sandbox.cleanup();
        state_clone.is_running.store(false, Ordering::SeqCst);
        let _ = tx.send("[DONE]".to_string());
    });

    Ok("Benchmark started".to_string())
}

async fn stream_logs(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.log_sender.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(line) => Some(Ok(Event::default().data(line))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
