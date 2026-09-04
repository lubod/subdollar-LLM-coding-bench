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
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::sleep;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::bench::BenchmarkRunner;
use crate::config::TaskType;
use crate::cost::ModelPricing;
use crate::report::{
    BenchmarkRunResult, EnvironmentInfo, FileInfo, LeaderboardManager, PublishResult, RunArchiver,
    RunManifest, RunPublisher, RunTokenUsage, SummaryGenerator,
};
use crate::sandbox::{AgentExecutionLimits, OmpRunner, OmpSessionStats, SandboxManager};
use crate::verifier::{HttpVerifier, RedisVerifier};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveRunInfo {
    pub run_id: String,
    pub model: String,
    pub task: String,
    pub effort: String,
    pub started_at: String,
}

#[derive(Clone)]
pub struct AppState {
    pub log_sender: broadcast::Sender<String>,
    pub is_running: Arc<AtomicBool>,
    pub current_run: Arc<RwLock<Option<ActiveRunInfo>>>,
    pub log_buffer: Arc<RwLock<Vec<String>>>,
}

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub is_running: bool,
    pub current_run: Option<ActiveRunInfo>,
}

async fn get_status(State(state): State<AppState>) -> Json<StatusResponse> {
    let is_running = state.is_running.load(Ordering::SeqCst);
    let current_run = state.current_run.read().unwrap().clone();
    Json(StatusResponse {
        is_running,
        current_run,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunRequest {
    pub model: String,
    pub task: String,
    pub api_key: Option<String>,
    pub budget_usd: f64,
    pub max_turns: u32,
    pub eval_only: bool,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub timeout_min: Option<u64>,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishRequest {
    pub message: Option<String>,
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
    pub fn build_app(state: AppState) -> Router {
        Router::new()
            .route("/", get(serve_index))
            .route("/api/models", get(get_models))
            .route("/api/prompt/:task", get(get_prompt).post(save_prompt))
            .route("/api/leaderboard", get(get_leaderboard))
            .route("/api/status", get(get_status))
            .route("/api/run", post(start_run))
            .route("/api/stream", get(stream_logs))
            .route("/api/console", get(get_active_console))
            .route("/api/runs", get(get_runs))
            .route("/api/runs/:id", get(get_run_detail))
            .route("/api/runs/:id/log", get(get_run_log))
            .route("/api/runs/:id/files", get(get_run_files))
            .route("/api/runs/:id/file", get(get_run_file))
            .route("/api/runs/:id/publish", post(publish_run))
            .route("/api/summary", get(get_summary).post(regenerate_summary))
            .route("/api/env", get(get_env))
            .with_state(state)
    }

    pub async fn start(host: &str, port: u16) -> anyhow::Result<()> {
        let (log_sender, _) = broadcast::channel(500);
        let state = AppState {
            log_sender,
            is_running: Arc::new(AtomicBool::new(false)),
            current_run: Arc::new(RwLock::new(None)),
            log_buffer: Arc::new(RwLock::new(Vec::new())),
        };

        let app = Self::build_app(state);

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
    let api_key = params.get("key")
        .filter(|k| !k.trim().is_empty())
        .cloned()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok().filter(|k| !k.trim().is_empty()));
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

async fn get_active_console(State(state): State<AppState>) -> Response {
    let buf = state.log_buffer.read().unwrap();
    buf.join("\n").into_response()
}

async fn publish_run(
    AxumPath(run_id): AxumPath<String>,
    payload: Option<Json<PublishRequest>>,
) -> Result<Json<PublishResult>, Response> {
    let repo_root = Path::new(".");
    let runs_dir = RunArchiver::resolve_runs_dir();
    let results_dir = PathBuf::from(resolve_results_dir());
    let custom_msg = payload.and_then(|Json(p)| p.message);

    match RunPublisher::publish_run(
        repo_root,
        &runs_dir,
        &results_dir,
        &run_id,
        custom_msg.as_deref(),
    ) {
        Ok(res) => Ok(Json(res)),
        Err(e) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to publish run: {}", e),
        ).into_response()),
    }
}

async fn get_summary() -> Response {
    let summary_path = Path::new("SUMMARY.md");
    let fallback = Path::new("/home/ubuntu/subdollar-LLM-coding-bench/SUMMARY.md");
    let target = if summary_path.exists() {
        summary_path
    } else if fallback.exists() {
        fallback
    } else {
        let repo_root = Path::new(".");
        let runs_dir = RunArchiver::resolve_runs_dir();
        let _ = SummaryGenerator::update_summary_file(repo_root, &runs_dir);
        if summary_path.exists() {
            summary_path
        } else {
            fallback
        }
    };

    match fs::read_to_string(target) {
        Ok(content) => content.into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "SUMMARY.md not found").into_response(),
    }
}

async fn regenerate_summary() -> Response {
    let repo_root = Path::new(".");
    let runs_dir = RunArchiver::resolve_runs_dir();
    match SummaryGenerator::update_summary_file(repo_root, &runs_dir) {
        Ok(p) => match fs::read_to_string(p) {
            Ok(content) => content.into_response(),
            Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Read error: {}", e)).into_response(),
        },
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to generate summary: {}", e)).into_response(),
    }
}

async fn get_env() -> Json<EnvironmentInfo> {
    Json(EnvironmentInfo::detect())
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

        let effort_setting = req.effort.clone().unwrap_or_else(|| "auto".to_string());

        // Record active run & clear log buffer
        *state_clone.current_run.write().unwrap() = Some(ActiveRunInfo {
            run_id: run_id.clone(),
            model: req.model.clone(),
            task: req.task.clone(),
            effort: effort_setting.clone(),
            started_at: started_at.clone(),
        });
        state_clone.log_buffer.write().unwrap().clear();

        let log_buf_arc = state_clone.log_buffer.clone();
        let tx_log = tx.clone();
        let mut console_buffer: Vec<String> = Vec::new();
        let log = move |buf: &mut Vec<String>, msg: String| {
            let ts = Utc::now().format("%H:%M:%S").to_string();
            let formatted = format!("[{}] {}", ts, msg);
            buf.push(formatted.clone());
            log_buf_arc.write().unwrap().push(formatted.clone());
            let _ = tx_log.send(formatted);
        };

        let timeout_m = req.timeout_min.unwrap_or(15);
        let limits = AgentExecutionLimits {
            max_turns: if req.max_turns > 0 { Some(req.max_turns) } else { None },
            max_budget_usd: if req.budget_usd > 0.0 { Some(req.budget_usd) } else { None },
            timeout_seconds: if timeout_m > 0 { Some(timeout_m * 60) } else { None },
        };

        log(&mut console_buffer, format!("========================================================="));
        log(&mut console_buffer, format!(">>> Benchmark Run: {}", run_id));
        log(&mut console_buffer, format!("    Model:   {}", req.model));
        log(&mut console_buffer, format!("    Effort:  {}", effort_setting));
        log(&mut console_buffer, format!("    Task:    {}", req.task));
        log(&mut console_buffer, format!("    Budget:  ${:.2} USD | Max Turns: {} | Timeout: {} min", req.budget_usd, req.max_turns, timeout_m));
        log(&mut console_buffer, format!("========================================================="));

        let effective_api_key = req.api_key
            .as_ref()
            .filter(|k| !k.trim().is_empty())
            .cloned()
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok().filter(|k| !k.trim().is_empty()));

        if let Some(ref k) = effective_api_key {
            std::env::set_var("OPENROUTER_API_KEY", k);
        }

        // Initial OpenRouter spend checkpoint
        let initial_spend = if let Some(ref key) = effective_api_key {
            ModelPricing::query_openrouter_key_usage(key).await
        } else {
            None
        };
        if let Some(init) = initial_spend {
            log(&mut console_buffer, format!("[ACCOUNT] OpenRouter initial key spend: ${:.4} USD", init));
        }

        let task_type = if req.task == "http" { TaskType::Http } else { TaskType::Redis };
        let work_path = Path::new("./workspace");
        if !req.eval_only {
            let _ = fs::remove_dir_all(work_path);
        }
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
        let mut agent_failed = false;
        let (omp_stats, prompt_tokens, cached_tokens, completion_tokens) = if !req.eval_only {
            log(&mut console_buffer, format!("[OMP] Spawning OMP agent inside isolated Docker sandbox ('subdollar-sandbox') with model '{}' (Effort: {})...", req.model, effort_setting));
            let tx_sub = tx.clone();
            let log_buf_sub = state_clone.log_buffer.clone();
            match OmpRunner::run_agent_with_logger(
                &req.model,
                &prompt_content,
                work_path,
                effective_api_key.as_deref(),
                limits,
                Some(&effort_setting),
                move |line| {
                    let ts = Utc::now().format("%H:%M:%S").to_string();
                    let formatted = format!("[{}] [OMP] {}", ts, line);
                    log_buf_sub.write().unwrap().push(formatted.clone());
                    let _ = tx_sub.send(formatted);
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
                    log(&mut console_buffer, format!("[OMP ERROR] Agent execution failed: {}", e));
                    agent_failed = true;
                    (OmpSessionStats::default(), 0, 0, 0)
                }
            }
        } else {
            log(&mut console_buffer, "[OMP] Skipped (--eval-only)".to_string());
            (OmpSessionStats::default(), 0, 0, 0)
        };

        // Check if candidate produced start.sh or Dockerfile (root or nested)
        let has_runnable = sandbox.ensure_runnable_candidate(work_path).unwrap_or(false);
        if !agent_failed && !has_runnable {
            log(&mut console_buffer, "[SANDBOX] Candidate did not create 'start.sh' or 'Dockerfile' in workspace. Skipping verification.".to_string());
            agent_failed = true;
        }

        let target_port = match task_type { TaskType::Redis => 6379, TaskType::Http => 8080 };
        let mut throughput = None;

        let (pass_rate, passed_stages, total_stages, stage_results) = if agent_failed {
            log(&mut console_buffer, "[TEST] Skipped: agent did not produce an executable server.".to_string());
            (0.0, 0, 4, Vec::new())
        } else {
            // 4. Start candidate container
            log(&mut console_buffer, "[SANDBOX] Launching candidate clone in isolated Docker container...".to_string());
            let _ = sandbox.start_candidate_in_docker(work_path, target_port);

            // Active TCP Port Health Check
            log(&mut console_buffer, format!("[SETUP] Polling port {} for candidate readiness (timeout: 30s)...", target_port));
            let ready = sandbox.wait_for_port(target_port, 30).await;
            if ready {
                log(&mut console_buffer, format!("[SETUP] Candidate server is online and accepting connections on port {}!", target_port));
            } else {
                log(&mut console_buffer, format!("[WARN] Candidate port {} did not respond within 30s. Proceeding to tests...", target_port));
            }

            // 5. Verification Test Suite
            log(&mut console_buffer, "[TEST] Running Protocol Verification Test Suite...".to_string());
            let (pr, ps, ts, sr) = match task_type {
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
            if pr < 100.0 {
                let container_logs = sandbox.get_candidate_logs();
                log(&mut console_buffer, "[DIAGNOSTICS] Candidate container runtime output:".to_string());
                for l in container_logs.lines() {
                    log(&mut console_buffer, format!("  | {}", l));
                }
            }

            // 6. Concurrency stress test in Docker
            if pr >= 75.0 {
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

            (pr, ps, ts, sr)
        };

        // 7. Cost & Metrics Accounting
        let pricing = ModelPricing::for_model(&req.model);
        let breakdown = pricing.compute_cost_with_cache(prompt_tokens, cached_tokens, completion_tokens);

        // Check OpenRouter live spending delta if available
        let mut live_spend_delta = None;
        if let (Some(key), Some(init)) = (&effective_api_key, initial_spend) {
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
            status: if pass_rate == 100.0 {
                "completed".to_string()
            } else if agent_failed {
                "agent_failed".to_string()
            } else {
                "failed_tests".to_string()
            },
            language: lang.clone(),
            effort: Some(effort_setting.clone()),
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
            env: None,
            git_commit: None,
            is_published: None,
        };

        // Archive complete run
        let runs_dir = RunArchiver::resolve_runs_dir();
        let full_console_log = state_clone.log_buffer.read().unwrap().join("\n");
        let _ = RunArchiver::archive_run(&runs_dir, &manifest, work_path, &full_console_log);
        log(&mut console_buffer, format!("[ARCHIVE] Run trace, workspace files & manifest archived to runs/{}/", run_id));

        // Save to leaderboard
        let result = BenchmarkRunResult {
            id: run_id,
            model: req.model.clone(),
            task: req.task.clone(),
            language: lang.clone(),
            effort: Some(effort_setting.clone()),
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
            "Results: Lang={}, Effort={}, Pass Rate={:.1}%, Cost=${:.4}, Savings={:.1}%, Efficiency={:.1}",
            lang, effort_setting, pass_rate, cost_usd, breakdown.savings_percent, efficiency_score
        ));
        log(&mut console_buffer, "=========================================================".to_string());

        sandbox.cleanup();
        *state_clone.current_run.write().unwrap() = None;
        state_clone.is_running.store(false, Ordering::SeqCst);
        let _ = tx.send("[DONE]".to_string());
    });

    Ok("Benchmark started".to_string())
}

async fn stream_logs(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.log_sender.subscribe();

    // 1. Snapshot historical logs from active/last run
    let initial_events: Vec<Event> = {
        let buf = state.log_buffer.read().unwrap();
        buf.iter().map(|line: &String| Event::default().data(line.clone())).collect()
    };
    let initial_stream = tokio_stream::iter(initial_events.into_iter().map(Ok));

    // 2. Stream live broadcast logs
    let live_stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(line) => Some(Ok(Event::default().data(line))),
        Err(_) => None,
    });

    let combined = initial_stream.chain(live_stream);
    Sse::new(combined).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn create_test_state() -> AppState {
        let (log_sender, _) = broadcast::channel(100);
        AppState {
            log_sender,
            is_running: Arc::new(AtomicBool::new(false)),
            current_run: Arc::new(RwLock::new(None)),
            log_buffer: Arc::new(RwLock::new(Vec::new())),
        }
    }

    #[tokio::test]
    async fn test_web_routes_lifecycle() {
        let state = create_test_state();
        state.log_buffer.write().unwrap().push("[12:00:00] Initial log line".to_string());

        let app = UiServer::build_app(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", port);

        // 1. GET /
        let res = client.get(&base).send().await.unwrap();
        assert_eq!(res.status(), 200);
        let html = res.text().await.unwrap();
        assert!(html.contains("SubDollarBench"));

        // 2. GET /api/status
        let res = client.get(format!("{}/api/status", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);
        let status_json: StatusResponse = res.json().await.unwrap();
        assert!(!status_json.is_running);
        assert!(status_json.current_run.is_none());

        // 3. GET /api/env
        let res = client.get(format!("{}/api/env", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);
        let env_json: EnvironmentInfo = res.json().await.unwrap();
        assert!(!env_json.os.is_empty());

        // 4. GET /api/prompt/redis
        let res = client.get(format!("{}/api/prompt/redis", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // 5. POST /api/prompt/test_task
        let res = client.post(format!("{}/api/prompt/test_task", base)).body("# Test Task").send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.text().await.unwrap(), "Prompt saved successfully");

        // 6. GET /api/leaderboard
        let res = client.get(format!("{}/api/leaderboard", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // 7. GET /api/console
        let res = client.get(format!("{}/api/console", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.text().await.unwrap().contains("Initial log line"));

        // 8. GET /api/runs
        let res = client.get(format!("{}/api/runs", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // 9. GET /api/runs/nonexistent_xyz
        let res = client.get(format!("{}/api/runs/nonexistent_xyz", base)).send().await.unwrap();
        assert_eq!(res.status(), 404);

        // 10. GET /api/runs/nonexistent_xyz/log
        let res = client.get(format!("{}/api/runs/nonexistent_xyz/log", base)).send().await.unwrap();
        assert_eq!(res.status(), 404);

        // 11. GET /api/runs/nonexistent_xyz/files
        let res = client.get(format!("{}/api/runs/nonexistent_xyz/files", base)).send().await.unwrap();
        assert_eq!(res.status(), 404);

        // 12. GET /api/runs/nonexistent_xyz/file?file=abc.txt
        let res = client.get(format!("{}/api/runs/nonexistent_xyz/file?file=abc.txt", base)).send().await.unwrap();
        assert_eq!(res.status(), 404);

        // 13. GET /api/summary
        let res = client.get(format!("{}/api/summary", base)).send().await.unwrap();
        assert!(res.status() == 200 || res.status() == 404);

        // 14. POST /api/summary
        let res = client.post(format!("{}/api/summary", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // 15. POST /api/run conflict test
        state.is_running.store(true, Ordering::SeqCst);
        let run_req = RunRequest {
            model: "test_model".to_string(),
            task: "redis".to_string(),
            api_key: None,
            budget_usd: 0.10,
            max_turns: 5,
            eval_only: true,
            effort: Some("low".to_string()),
            timeout_min: Some(1),
        };
        let res = client.post(format!("{}/api/run", base)).json(&run_req).send().await.unwrap();
        assert_eq!(res.status(), 409);

        // 16. POST /api/run eval_only mode
        state.is_running.store(false, Ordering::SeqCst);
        let eval_req = RunRequest {
            model: "google/gemini-2.5-flash".to_string(),
            task: "redis".to_string(),
            api_key: None,
            budget_usd: 0.10,
            max_turns: 1,
            eval_only: true,
            effort: Some("low".to_string()),
            timeout_min: Some(1),
        };
        let res = client.post(format!("{}/api/run", base)).json(&eval_req).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // Allow background thread to process
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !state.is_running.load(Ordering::SeqCst) {
                break;
            }
        }

        // 17. GET /api/models
        let res = client.get(format!("{}/api/models", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // 18. GET /api/stream
        let res = client.get(format!("{}/api/stream", base)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // 19. Tests with a valid archived run
        let runs_dir = RunArchiver::resolve_runs_dir();
        let test_run_id = "test_archived_run_server_123";
        let test_run_dir = runs_dir.join(test_run_id);
        let test_ws = test_run_dir.join("workspace");
        let _ = fs::create_dir_all(&test_ws);
        fs::write(test_ws.join("main.rs"), "fn main() { println!(\"hello\"); }").unwrap();
        fs::write(test_run_dir.join("console.log"), "Mock console output for test").unwrap();
        let test_manifest = RunManifest {
            run_id: test_run_id.to_string(),
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
            throughput_req_sec: Some(50000.0),
            tokens: RunTokenUsage {
                prompt_tokens: 1000,
                cached_tokens: 500,
                completion_tokens: 200,
                total_tokens: 1200,
            },
            cost_usd: 0.05,
            savings_percent: 25.0,
            efficiency_score: 20.0,
            files: Vec::new(),
            env: None,
            git_commit: None,
            is_published: None,
        };
        fs::write(test_run_dir.join("manifest.json"), serde_json::to_string_pretty(&test_manifest).unwrap()).unwrap();

        // Check GET /api/runs/:id
        let res = client.get(format!("{}/api/runs/{}", base, test_run_id)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // Check GET /api/runs/:id/log
        let res = client.get(format!("{}/api/runs/{}/log", base, test_run_id)).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.text().await.unwrap().contains("Mock console output"));

        // Check GET /api/runs/:id/files
        let res = client.get(format!("{}/api/runs/{}/files", base, test_run_id)).send().await.unwrap();
        assert_eq!(res.status(), 200);

        // Check GET /api/runs/:id/file?file=main.rs
        let res = client.get(format!("{}/api/runs/{}/file?file=main.rs", base, test_run_id)).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.text().await.unwrap().contains("fn main()"));

        // Check POST /api/runs/:id/publish (tests publish error handling without committing to git)
        let pub_req = PublishRequest { message: Some("Publish test".to_string()) };
        let res = client.post(format!("{}/api/runs/nonexistent_xyz/publish", base)).json(&pub_req).send().await.unwrap();
        assert_eq!(res.status(), 500);

        // 20. POST /api/run with runnable start.sh to test Docker candidate branch
        let ws_dir = Path::new("./workspace");
        let _ = fs::create_dir_all(ws_dir);
        fs::write(ws_dir.join("start.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(ws_dir.join("start.sh"), fs::Permissions::from_mode(0o755));
        }

        let run_candidate_req = RunRequest {
            model: "google/gemini-2.5-flash".to_string(),
            task: "http".to_string(),
            api_key: None,
            budget_usd: 0.10,
            max_turns: 1,
            eval_only: true,
            effort: Some("low".to_string()),
            timeout_min: Some(1),
        };
        let res = client.post(format!("{}/api/run", base)).json(&run_candidate_req).send().await.unwrap();
        assert_eq!(res.status(), 200);

        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !state.is_running.load(Ordering::SeqCst) {
                break;
            }
        }

        // Cleanup test directory and generated test runs
        let _ = fs::remove_dir_all(&test_run_dir);
        let _ = fs::remove_dir_all(ws_dir);
        let _ = fs::remove_dir_all("tasks/test_task");
        let _ = fs::remove_dir_all("/home/ubuntu/subdollar-LLM-coding-bench/tasks/test_task");

        // Clean any gemini/test artifacts from runs/ and results/
        if let Ok(entries) = fs::read_dir(&runs_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        let r_dir = PathBuf::from(resolve_results_dir());
        if let Ok(entries) = fs::read_dir(&r_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("gemini-2.5-flash") || name.starts_with("test_") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    #[test]
    fn test_helpers_resolve() {
        let task_p = resolve_task_path("redis");
        assert!(task_p.to_string_lossy().contains("redis"));
        let res_dir = resolve_results_dir();
        assert!(res_dir.contains("results"));
    }
}
