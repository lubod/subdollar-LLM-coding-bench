# 🏆 subdollar-LLM-coding-bench

> **Autonomous, Under-$1 Systems Engineering Benchmark for Budget & Flash LLMs**

`subdollar-LLM-coding-bench` is a reproducible, fully isolated benchmarking framework designed to evaluate budget / flash LLMs (< $1.00 / 1M tokens) on real-world systems programming tasks.

Instead of evaluating trivial single-function code snippets ([HumanEval](https://github.com/openai/human-eval)) or relying on heavyweight, multi-hour git patches on legacy repos ([SWE-bench](https://www.swebench.com/)), this benchmark challenges models to **autonomously design, implement, debug, package, and optimize complete protocol-compliant network servers from scratch**.

---

## 🏗️ System Architecture

```text
+-----------------------------------------------------------------------------------------+
|                                    HOST ENVIRONMENT                                     |
|                                                                                         |
|   +--------------------------+                      +-------------------------------+   |
|   |   Web UI / CLI Harness   | <==================> |      Run Archiver & Git       |   |
|   | (Axum SSE, Tokio, Clap)  |                      | (runs/<id>/, SUMMARY, results)|   |
|   +------------+-------------+                      +-------------------------------+   |
|                |                                                                        |
+----------------|------------------------------------------------------------------------+
                 | Docker Socket Management
                 v
+-----------------------------------------------------------------------------------------+
|                              DOCKER SANDBOX ISOLATION LAYER                             |
|                                                                                         |
|  +---------------------------+     tcp:6380 / 8081      +----------------------------+  |
|  |     OMP Agent Sandbox     | -----------------------> |  Ground-Truth Reference    |  |
|  |   'subdollar-sandbox'     |  (Inspects reference     | (Official Redis / Nginx)   |  |
|  | (Rust, Go, Node, Python,  |   behavior via bash)     +----------------------------+  |
|  |  GCC, Bun, OMP runtime)   |                                                          |
|  +-------------+-------------+                                                          |
|                | produces candidate code                                                |
|                v                                                                        |
|  +---------------------------+                          +----------------------------+  |
|  |    Candidate Container    | <----------------------- |    Verification Harness    |  |
|  |    'subdollar-candidate'  |     tcp:6379 / 8080      | (Redis RESP2 / HTTP 1.1)   |  |
|  | (Runs model's Dockerfile  |                          |            +               |  |
|  |   or ./start.sh binary)   |                          | (redis-benchmark / wrk)    |  |
|  +---------------------------+                          +----------------------------+  |
+-----------------------------------------------------------------------------------------+
```

---

## 🌟 Core Innovations

1. **Model Selects the Language & Architecture:**
   The model has complete freedom to choose any programming language (Go, Rust, Python, Node.js, C/C++) and architecture. This directly tests SWE pragmatism—balancing execution speed, memory safety, concurrency models, and compiler feedback loops.

2. **Dual-Tier Docker Isolation & Controlled Sandboxing:**
   - **OMP Agent Sandbox (`subdollar-sandbox`):** The LLM operates strictly inside an isolated Docker container with pre-installed toolchains (Rust, Go, Node, Python, Clang/GCC, Bun) and workspace volume mounting.
   - **Ground-Truth Reference:** An official reference server (`redis:alpine` on port 6380, reference web server on port 8081) runs concurrently. The agent can issue black-box requests (e.g. `redis-cli -p 6380` or `curl http://localhost:8081`) to inspect actual protocol responses.
   - **Candidate Isolation (`subdollar-candidate`):** The generated code is compiled and launched in a clean container, mapping candidate ports (6379 for Redis, 8080 for HTTP).

3. **Packaging Freedom (`Dockerfile` or `./start.sh`):**
   The harness automatically detects if the model built a standalone `Dockerfile` (root or nested project directory) and builds a clean container. If no Dockerfile is found, it automatically wraps `./start.sh` inside the multi-language sandbox environment.

4. **Multi-Stage Protocol Verification:**
   Zero-flake, byte-level conformance suites verify protocol compliance without third-party abstraction layers. If tests fail, container runtime logs and diagnostics are automatically captured into the audit trail.

5. **Load & Stress Benchmarking:**
   Servers that pass verification are subjected to automated concurrency stress testing using official benchmark tools (`redis-benchmark` and `wrk`), recording requests per second.

6. **Formula & Live-Spend Cost Accounting:**
   Tracks token consumption with input, output, and context caching discounts. For OpenRouter API keys, directly queries the OpenRouter management API before and after each run to record verified live USD billing deltas.

7. **Production Web GUI & Git Publisher:**
   Includes a rich single-page web application featuring live Server-Sent Events (SSE) log streaming, responsive run tables, a tabbed modal code inspector, interactive prompt markdown editor, and automated git publishing to `SUMMARY.md`.

---

## 🎯 Supported Tasks

| Task | Description | Stages Verified | Concurrency Stress Test |
| :--- | :--- | :--- | :--- |
| **`redis`** | In-memory key-value database implementing the RESP2 wire protocol | 1. `PING` / `ECHO` Handshake<br>2. `SET`, `GET`, `DEL`, `EXISTS`<br>3. `SET ... PX <ms>` (TTL expiration & passive eviction)<br>4. `INCR` / `DECR` Atomic Counters | `redis-benchmark -p 6379 -t set,get -n 5000 -c 10 -q` |
| **`http`** | RFC 7230 / RFC 7231 compliant HTTP/1.1 web server | 1. Root `GET /` (200 OK)<br>2. 404 Not Found Handling<br>3. `GET /echo/{str}` with dynamic `Content-Length`<br>4. `GET /user-agent` Header Reflection<br>5. `POST` & `GET /files/{name}` File Persistence | `wrk -t2 -c20 -d3s http://127.0.0.1:8080/` |

---

## 🚀 Quick Start

### 1. Prerequisites
- **Docker**: Docker CE 24+ installed and running with socket access.
- **Rust Toolchain**: Rust 1.80+ (`cargo`, `rustc`).
- **OpenRouter API Key**: Export `OPENROUTER_API_KEY` in your environment.

### 2. Build the Multi-Language Sandbox Image
```bash
git clone https://github.com/username/subdollar-LLM-coding-bench.git
cd subdollar-LLM-coding-bench
docker build -t subdollar-sandbox -f Dockerfile.sandbox .
```

### 3. Launch the Interactive Web UI (Recommended)
```bash
cargo run --release -- ui --port 3000
```
> ⚠️ **Security Warning**: The Web UI does not enforce authentication. Always bind to loopback (`127.0.0.1`, default). Do not bind to `0.0.0.0` or expose the UI port on untrusted networks, as anyone with network access could trigger benchmark runs, spend your API credits, or modify task prompts.

Open `http://localhost:3000` in your browser. From the UI, you can:
- Browse all models on OpenRouter priced below $1.00 / 1M tokens.
- Choose between `redis` and `http` tasks.
- Configure reasoning effort (`auto`, `max`, `high`, `medium`, `low`, `off`).
- Watch live agent thoughts, bash actions, and verification stages stream in real-time.
- Inspect candidate source code in the full-screen modal code browser.
- Publish verified runs directly to Git and update `SUMMARY.md` with one click.

---

### 4. Headless CLI Usage

#### Run an Autonomous Benchmark
```bash
export OPENROUTER_API_KEY="sk-or-v1-your-key-here"

# Run Redis benchmark with auto reasoning effort
cargo run --release -- run \
  --task redis \
  --model openrouter/deepseek/deepseek-chat \
  --budget-usd 0.50 \
  --max-turns 15

# Run HTTP benchmark with max reasoning effort
cargo run --release -- run \
  --task http \
  --model openrouter/google/gemini-2.5-flash \
  --effort max \
  --budget-usd 0.50
```

#### CLI Options Reference
```text
Usage: subdollar-bench run [OPTIONS] --model <MODEL>

Options:
  -m, --model <MODEL>            OpenRouter model ID (e.g. openrouter/deepseek/deepseek-chat)
  -t, --task <TASK>              Benchmark task [possible values: redis, http, dns] [default: redis]
  -e, --effort <EFFORT>          Reasoning effort: auto, max, high, medium, low, off [default: auto]
  -b, --budget-usd <BUDGET_USD>  Maximum cost allowance in USD [default: 0.50]
      --max-turns <MAX_TURNS>    Maximum agent conversation turns [default: 15]
      --timeout-min <MINUTES>    Execution timeout in minutes [default: 15]
      --api-key <KEY>            OpenRouter API key (overrides OPENROUTER_API_KEY env var)
      --workdir <DIR>            Candidate workspace output directory [default: ./workspace]
      --eval-only                Skip agent generation; evaluate existing code in workdir
      --trials <TRIALS>          Number of repeated trials for statistical variance (Pass@k) [default: 1]
      --no-save                  Do not save results or archive run artifacts to disk [default: false]
```

#### Direct Protocol Evaluation (Standalone Mode)
Test an already running server on a specific port without running the agent:
```bash
# Verify Redis server on port 6379
cargo run --release -- eval --task redis --port 6379

# Verify HTTP server on port 8080
cargo run --release -- eval --task http --port 8080

# Verify DNS server on port 5353
cargo run --release -- eval --task dns --port 5353
```

#### Inspect Leaderboard
```bash
cargo run --release -- leaderboard
```

#### Publish Run to Git & Update Global Leaderboard
```bash
cargo run --release -- publish --run-id redis_openrouter_deepseek_deepseek-chat_20260904_120000
```

#### Regenerate SUMMARY.md
```bash
cargo run --release -- summary
```

---

## 💰 Pricing Engine & Efficiency Scoring

SubDollarBench ranks models primarily by their **Engineering Efficiency Score**:

$$\text{Efficiency Score} = \frac{\text{Pass Rate (\%)}}{\text{Cost (Cents)}} = \frac{\text{Pass Rate (\%)}}{\text{Cost (USD)} \times 100}$$

### Example Rankings:
- Model A: 100% pass rate at $0.010 USD (1.0¢) $\rightarrow$ **100.0 pts/¢**
- Model B: 100% pass rate at $0.005 USD (0.5¢) $\rightarrow$ **200.0 pts/¢**
- Model C: 75% pass rate at $0.005 USD (0.5¢) $\rightarrow$ **150.0 pts/¢**

### Supported Cost Calculation:
1. **Live OpenRouter Accounting:** If an API key is provided, the harness queries the `/api/v1/auth/key` endpoint before and after the run to verify the exact billing delta deducted by OpenRouter.
2. **Context Cache Awareness:** For token-based formula fallback, cached prompt tokens are discounted by 50% to 90% depending on model provider caching rules.

---

## 📁 Run Traceability & Archival Structure

Every benchmark run generates a self-contained, reproducible archive in `runs/<run_id>/`:

```text
runs/<run_id>/
├── manifest.json      # Structured metrics: tokens, cost, pass rate, stages, duration
├── console.log        # Full console log: agent thoughts, bash tool invocations, compiler logs
├── env.json           # Hardware environment snapshot: CPU, RAM, OS, kernel, Docker & Rust versions
├── README.md          # Markdown report summarizing the run
└── workspace/         # Exact candidate source code produced by the LLM
    ├── Dockerfile     # (or start.sh)
    └── ...            # Source files (main.rs, main.go, server.py, etc.)
```

---

## 🖥️ Reproduction & Submission Workflow

1. Fork and clone this repository.
2. Build the Docker sandbox: `docker build -t subdollar-sandbox -f Dockerfile.sandbox .`
3. Launch the Web UI (`cargo run --release -- ui`) or run via CLI.
4. When a run passes, publish it:
   ```bash
   cargo run --release -- publish --run-id <run_id>
   ```
5. Submit a Pull Request with your new `runs/<run_id>/`, updated `results/`, and regenerated `SUMMARY.md`.

---

## 📜 License
Apache-2.0 or MIT.
