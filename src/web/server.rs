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
use crate::report::{BenchmarkRunResult, LeaderboardManager};
use crate::sandbox::{OmpRunner, SandboxManager};
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
            .with_state(state);

        let addr = format!("{}:{}", host, port);
        println!("🚀 SubDollarBench GUI listening on: http://{}", addr);
        println!("   Local URL:  http://localhost:{}", port);
        println!("   LXD Dev IP: http://10.138.94.191:{}", port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

async fn serve_index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

async fn get_models(
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<SubDollarModel>>, (axum::http::StatusCode, String)> {
    let client = reqwest::Client::new();
    let mut req = client.get("https://openrouter.ai/api/v1/models");

    if let Some(key) = params.get("api_key") {
        if !key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", key.trim()));
        }
    }

    let resp = req.send().await.map_err(|e| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("Failed to fetch OpenRouter models: {}", e),
        )
    })?;

    let data = resp.json::<OpenRouterResponse>().await.map_err(|e| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("Failed to parse OpenRouter response: {}", e),
        )
    })?;

    let max_price = params
        .get("max_price")
        .and_then(|p| p.parse::<f64>().ok())
        .unwrap_or(1.0);

    let mut filtered: Vec<SubDollarModel> = data
        .data
        .into_iter()
        .filter_map(|item| {
            let pricing = item.pricing?;
            let p_str = pricing.prompt?;
            let c_str = pricing.completion?;

            let p_raw: f64 = p_str.parse().ok()?;
            let c_raw: f64 = c_str.parse().ok()?;

            let p_m = p_raw * 1_000_000.0;
            let c_m = c_raw * 1_000_000.0;

            if p_m >= 0.0 && c_m >= 0.0 && p_m <= max_price && c_m <= max_price {
                Some(SubDollarModel {
                    id: item.id,
                    name: item.name,
                    prompt_price_per_m: p_m,
                    completion_price_per_m: c_m,
                    context_length: item.context_length.unwrap_or(0),
                    created: item.created.unwrap_or(0),
                })
            } else {
                None
            }
        })
        .collect();

    filtered.sort_by(|a, b| (b.created).cmp(&a.created));

    Ok(Json(filtered))
}

async fn get_prompt(AxumPath(task): AxumPath<String>) -> Response {
    let p = resolve_task_path(&task);
    match fs::read_to_string(p) {
        Ok(c) => c.into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "Task prompt not found").into_response(),
    }
}

async fn save_prompt(AxumPath(task): AxumPath<String>, body: String) -> Response {
    let p = resolve_task_path(&task);
    match fs::write(p, body) {
        Ok(_) => "Prompt saved successfully".into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_leaderboard() -> Json<Vec<BenchmarkRunResult>> {
    let r_dir = resolve_results_dir();
    let results = LeaderboardManager::load_all(&r_dir);
    Json(results)
}

async fn start_run(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Result<String, (axum::http::StatusCode, String)> {
    if state.is_running.load(Ordering::SeqCst) {
        return Err((
            axum::http::StatusCode::CONFLICT,
            "A benchmark run is already in progress.".to_string(),
        ));
    }

    state.is_running.store(true, Ordering::SeqCst);
    let state_clone = state.clone();

    tokio::spawn(async move {
        let tx = state_clone.log_sender.clone();
        let _ = tx.send(format!("🚀 Starting run for {} on task '{}'", req.model, req.task));

        // Initial OpenRouter spend checkpoint
        let initial_spend = if let Some(key) = &req.api_key {
            ModelPricing::query_openrouter_key_usage(key).await
        } else {
            None
        };
        if let Some(init) = initial_spend {
            let _ = tx.send(format!("[ACCOUNT] OpenRouter initial key spend: ${:.4} USD", init));
        }

        let task_type = if req.task == "http" { TaskType::Http } else { TaskType::Redis };
        let work_path = Path::new("./workspace");
        let _ = fs::create_dir_all(work_path);

        let sandbox = SandboxManager::new();

        // 1. Reference ground truth in Docker
        let _ = tx.send("[SETUP] Starting official reference server in Docker...".to_string());
        match task_type {
            TaskType::Redis => { let _ = sandbox.start_reference_redis(6380); }
            TaskType::Http => { let _ = sandbox.start_reference_http(8081); }
        }

        // 2. Read prompt
        let prompt_path = resolve_task_path(&req.task);
        let prompt_content = fs::read_to_string(&prompt_path).unwrap_or_default();

        // 3. Run OMP Agent
        let (prompt_tokens, cached_tokens, completion_tokens) = if !req.eval_only {
            let _ = tx.send(format!("[OMP] Spawning OMP agent with model '{}'...", req.model));
            match OmpRunner::run_agent(&req.model, &prompt_content, work_path, req.api_key.as_deref(), req.max_turns) {
                Ok(stats) => {
                    let _ = tx.send(format!(
                        "[OMP] Finished! Tokens: prompt={}, cached={}, completion={}",
                        stats.prompt_tokens, stats.cached_tokens, stats.completion_tokens
                    ));
                    (stats.prompt_tokens, stats.cached_tokens, stats.completion_tokens)
                }
                Err(e) => {
                    let _ = tx.send(format!("[OMP ERROR] {}", e));
                    (15_000, 10_000, 2_500)
                }
            }
        } else {
            let _ = tx.send("[OMP] Skipped (--eval-only)".to_string());
            (0, 0, 0)
        };

        // 4. Start candidate container
        let target_port = match task_type { TaskType::Redis => 6379, TaskType::Http => 8080 };
        let _ = tx.send("[SANDBOX] Launching candidate clone in isolated Docker container...".to_string());
        let _ = sandbox.start_candidate_in_docker(work_path, target_port);
        sleep(Duration::from_secs(3)).await;

        // 5. Verification Test Suite
        let _ = tx.send("[TEST] Running Protocol Verification Test Suite...".to_string());
        let (pass_rate, passed_stages, total_stages) = match task_type {
            TaskType::Redis => {
                let verifier = RedisVerifier::new(6379, Some(6380));
                let summary = verifier.run_all().await;
                for s in &summary.stages {
                    if s.passed {
                        let _ = tx.send(format!("  [PASS] {}", s.name));
                    } else {
                        let err_msg = s.error.as_deref().unwrap_or("unknown error");
                        let _ = tx.send(format!("  [FAIL] {} - Error: {}", s.name, err_msg));
                    }
                }
                (summary.pass_rate, summary.passed_count, summary.total_stages)
            }
            TaskType::Http => {
                let verifier = HttpVerifier::new(8080);
                let summary = verifier.run_all().await;
                for s in &summary.stages {
                    if s.passed {
                        let _ = tx.send(format!("  [PASS] {}", s.name));
                    } else {
                        let err_msg = s.error.as_deref().unwrap_or("unknown error");
                        let _ = tx.send(format!("  [FAIL] {} - Error: {}", s.name, err_msg));
                    }
                }
                (summary.pass_rate, summary.passed_count, summary.total_stages)
            }
        };

        // 6. Concurrency stress test in Docker
        let mut throughput = None;
        if pass_rate >= 75.0 {
            let _ = tx.send("[BENCH] Running Stress & Concurrency Benchmark in Docker...".to_string());
            match task_type {
                TaskType::Redis => {
                    if let Ok(tp) = BenchmarkRunner::run_redis_benchmark(6379) {
                        let _ = tx.send(format!("  Throughput: {:.0} req/sec", tp));
                        throughput = Some(tp);
                    }
                }
                TaskType::Http => {
                    if let Ok(tp) = BenchmarkRunner::run_wrk_benchmark(8080) {
                        let _ = tx.send(format!("  Throughput: {:.0} req/sec", tp));
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
            let _ = tx.send(format!("[BILLING] OpenRouter live verified cost: ${:.4} USD", delta));
        } else {
            let _ = tx.send(format!(
                "[BILLING] Formula Cost: ${:.4} USD (Prompt Cache Savings: {:.1}%)",
                breakdown.total_cost_usd, breakdown.savings_percent
            ));
        }

        let run_id = format!("{}_{}_{}", req.task, req.model.replace('/', "_"), Utc::now().format("%Y%m%d_%H%M%S"));
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
            timestamp: Utc::now().to_rfc3339(),
        };

        let r_dir = resolve_results_dir();
        let _ = LeaderboardManager::save_result(&r_dir, &result);

        let _ = tx.send("=========================================================".to_string());
        let _ = tx.send(format!(
            "Results: Lang={}, Pass Rate={:.1}%, Cost=${:.4}, Savings={:.1}%, Efficiency={:.1}",
            lang, pass_rate, cost_usd, breakdown.savings_percent, efficiency_score
        ));
        let _ = tx.send("=========================================================".to_string());

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
