# subdollar-LLM-coding-bench

> **Autonomous, Under-$1 System-Building Benchmark for Budget / Flash LLMs**

`subdollar-LLM-coding-bench` is a reproducible, open-source benchmarking framework designed to evaluate budget / flash LLMs (< $1.00 / 1M tokens) on real-world systems programming tasks.

Instead of testing isolated 1-function puzzles ([HumanEval](https://github.com/openai/human-eval)) or running heavy, expensive multi-turn git-diff patches ([SWE-bench](https://www.swebench.com/)), this benchmark challenges models to **build protocol-compliant network servers from scratch** (e.g. an in-memory Redis clone or an RFC-compliant HTTP/1.1 server).

---

## 🌟 Key Features

1. **Model Chooses the Language:**
   Models are given complete freedom to choose their implementation language (Go, Python, Rust, Node.js, C/C++). This directly evaluates engineering pragmatism (the tradeoff between development speed, concurrency, and compiler friction).

2. **100% Docker-Isolated Architecture:**
   - **Ground-Truth Services:** Official `redis:alpine` runs in Docker on port `6380` for live side-by-side inspection.
   - **Candidate Environment:** The model's server runs inside an isolated `subdollar-sandbox` container on port `6379`.
   - **Verification Tools:** `redis-benchmark` and `wrk` execute via ephemeral containers—zero host installation needed.

3. **OMP (Oh My Pi) Agent Harness:**
   Uses [Oh My Pi](https://github.com/can1357/oh-my-pi) as the headless agent runtime. OMP provides hashline-anchored editing and controlled bash execution so budget models can inspect ground truth on port `6380` and self-test before declaring completion.

4. **Zero-Flake Differential Verification:**
   The Rust test harness connects to the candidate container, checks RESP wire protocol serialization byte-for-byte across 4 progressive stages, and stress-tests concurrency via `redis-benchmark`.

5. **Strict Budget & Cost Tracking:**
   Computes exact USD cost per run using OpenRouter token pricing metadata. A full 5-stage benchmark run typically costs **between $0.005 and $0.03**.

---

## 📊 Live Leaderboard

| Model | Task | Language | Pass Rate | Throughput | Cost ($) | Efficiency (Score / ¢) |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| *(Run benchmark to populate)* | | | | | | |

---

## 🚀 Quick Start

### Prerequisites
- [Docker](https://docs.docker.com/get-docker/) installed and running
- [Rust](https://rustup.rs/) (1.80+)
- An [OpenRouter API Key](https://openrouter.ai/)

### 1. Build the Sandbox Image
```bash
docker build -t subdollar-sandbox -f Dockerfile.sandbox .
```

### 2. Run a Benchmark on a Budget Model
```bash
export OPENROUTER_API_KEY="your-openrouter-key"

# Benchmark Gemini 2.5 Flash on the Redis challenge
cargo run --release -- run \
  --model openrouter/google/gemini-2.5-flash \
  --task redis \
  --budget-usd 0.20

# Benchmark DeepSeek V3
cargo run --release -- run \
  --model openrouter/deepseek/deepseek-chat \
  --task redis \
  --budget-usd 0.20
```

### 3. Direct Protocol Evaluation (Manual Mode)
If you already have a server running on port `6379`:
```bash
cargo run --release -- eval --task redis --port 6379
```

### 4. View Stored Results & Leaderboard
```bash
cargo run --release -- leaderboard
```

---

## 🎯 Supported Tasks

| Task | Description | Stages Verified | Load Test |
| :--- | :--- | :--- | :--- |
| **`redis`** | In-memory key-value store with RESP protocol | 1. `PING`/`ECHO`<br>2. `SET`/`GET`/`DEL`/`EXISTS`<br>3. `SET ... PX <ms>` (TTL expiration)<br>4. `INCR`/`DECR` | `redis-benchmark` (5,000 reqs, 10 clients) |
| **`http`** | RFC-compliant HTTP/1.1 server | 1. Root `200 OK`<br>2. `404 Not Found`<br>3. `GET /echo/{str}`<br>4. `User-Agent` echo<br>5. `POST`/`GET /files/{name}` | `wrk` (concurrency stress) |

---

## 💰 Budget Model Target Costs (OpenRouter)

| Model | OpenRouter Slug | Input / 1M | Output / 1M | Est. Cost / Task |
| :--- | :--- | :--- | :--- | :--- |
| **Qwen 2.5 Coder 32B** | `openrouter/qwen/qwen-2.5-coder-32b-instruct` | $0.06 | $0.15 | **~$0.005** |
| **DeepSeek V3** | `openrouter/deepseek/deepseek-chat` | $0.14 | $0.28 | **~$0.008** |
| **Gemini 2.5 Flash** | `openrouter/google/gemini-2.5-flash` | $0.15 | $0.60 | **~$0.012** |
| **Llama 3.3 70B** | `openrouter/meta-llama/llama-3.3-70b-instruct` | $0.12 | $0.30 | **~$0.009** |
| **GPT-4o-mini** | `openrouter/openai/gpt-4o-mini` | $0.15 | $0.60 | **~$0.015** |

---

## 📂 Project Structure

```text
subdollar-LLM-coding-bench/
├── Cargo.toml               # Rust dependencies
├── Dockerfile.sandbox       # Multi-language runtime (Go, Python, Rust, Node, GCC) + OMP
├── tasks/
│   ├── redis/prompt.md      # Redis specification provided to the model
│   └── http/prompt.md       # HTTP/1.1 specification provided to the model
├── src/
│   ├── main.rs              # CLI entry point
│   ├── config.rs            # CLI args & task definitions
│   ├── sandbox/
│   │   ├── docker.rs        # Docker lifecycle for candidate & ground truth
│   │   └── omp.rs           # Headless OMP agent driver & token extractor
│   ├── verifier/
│   │   ├── redis.rs         # Raw TCP RESP frame verifier (Stages 1-4)
│   │   └── http.rs          # HTTP status & header verifier (Stages 1-5)
│   ├── bench/
│   │   └── mod.rs           # Dockerized redis-benchmark & wrk runner
│   ├── cost/
│   │   └── calculator.rs    # OpenRouter USD cost calculation
│   └── report/
│       └── leaderboard.rs   # UTF-8 table and Markdown exporter
└── results/                 # Persisted JSON evaluations per run
```

---

## 📜 License
MIT
