# 🏆 SubDollarBench: Autonomous Under-$1 LLM Systems Benchmark

> **Measuring under-$1/1M token LLMs on autonomous, production-grade SWE systems engineering tasks (Redis & HTTP/1.1) in isolated Docker sandboxes.**

*Last Updated: 2026-09-05 16:36:06 UTC*

## 📑 Quick Navigation
- [📊 Global Leaderboard](#-global-leaderboard)
- [⚡ Task 1: In-Memory Redis Server](#-task-1-in-memory-redis-server-task-redis)
- [🌐 Task 2: HTTP/1.1 Web Server](#-task-2-http11-web-server-task-http)
- [🖥️ Testbed Environment Specs](#%EF%B8%8F-benchmark-testbed-environment-specs)
- [🚀 How to Reproduce & Submit Results](#-how-to-reproduce--submit-your-results)

---

## 📊 Global Leaderboard

*No benchmark runs recorded yet. Run a benchmark to populate the leaderboard!*

---

## ⚡ Task 1: In-Memory Redis Server (`task: redis`)

The candidate LLM is instructed to build a production-grade, concurrent Redis clone from scratch.
- **Wire Protocol**: Full RESP2 parser handling pipelined TCP streams, bulk strings, integers, simple strings, and errors.
- **Supported Commands**: `PING`, `ECHO`, `SET` (with `PX` millisecond expiration, `EX`, `NX`, `XX`), `GET`, `DEL`, `EXISTS`, `INCR`, `DECR`, `COMMAND`, `QUIT`.
- **Packaging Freedom**: Working multi-stage `Dockerfile` (automatically built and containerized) or `./start.sh`.
- **Verification**: Automated raw-socket TCP verification test suite + `redis-benchmark -p 6379 -n 5000 -c 10 -t get,set -q`.

---

## 🌐 Task 2: HTTP/1.1 Web Server (`task: http`)

The candidate LLM is instructed to build an RFC 7230/7231 HTTP/1.1 web server from scratch.
- **Features**: `GET /` (200 OK), `404 Not Found Handling`, `GET /echo/{str}` with dynamic `Content-Length`, `GET /user-agent` header reflection, `POST` and `GET /files/{filename}` file persistence, HTTP/1.1 socket keep-alive reuse.
- **Packaging Freedom**: Working `Dockerfile` or executable `./start.sh`.
- **Verification**: 5-stage automated conformance suite + `wrk -t4 -c20 -d5s http://127.0.0.1:8080/`.

---

## 🖥️ Benchmark Testbed Environment Specs

All runs recorded in this repository were benchmarked under identical, isolated hardware and sandbox conditions:

| Component | Specification |
|:---|:---|
| **Operating System** | Ubuntu 24.04.3 LTS |
| **Kernel & Architecture** | Linux 7.0.0-30-generic (x86_64) |
| **CPU Model** | AMD Ryzen AI 9 HX 370 w/ Radeon 890M (24 vCPUs) |
| **System Memory** | 54.5 GiB |
| **Docker Runtime** | Docker version 29.1.3, build 29.1.3-0ubuntu3~24.04.2 |
| **Rust Version** | rustc 1.92.0 (ded5c06cf 2025-12-08) |
| **Python Version** | Python 3.12.3 |
| **Node.js Version** | v24.13.0 |
| **Git Baseline** | Commit `1b4d765` (branch `master`) |
| **Benchmark Captured** | 2026-09-05T16:36:06.118474034+00:00 |

---

## 🚀 How to Reproduce & Submit Your Results

Anyone can clone this repository, run benchmarks on their own models or hardware, and publish verified results.

### 1. Prerequisites
- **Docker**: Docker CE 24+ installed and running.
- **Rust**: Rust toolchain 1.80+ (`cargo`, `rustc`).
- **OMP Agent**: `oh-my-pi` installed (`bun install -g @oh-my-pi/pi-coding-agent`).
- **OpenRouter API Key**: Export `OPENROUTER_API_KEY` in your shell.

### 2. Clone the Repository
```bash
git clone https://github.com/username/subdollar-LLM-coding-bench.git
cd subdollar-LLM-coding-bench
export OPENROUTER_API_KEY="sk-or-v1-your-key-here"
```

### 3. Run the Benchmark

#### Option A: Interactive Web UI (Recommended)
```bash
cargo run --release -- ui --port 3000
# Open http://localhost:3000 in your browser
```
1. Select an under-$1/1M token model from the live OpenRouter catalog.
2. Select task (`redis` or `http`) and reasoning effort (`low`, `medium`, `high`, `max`).
3. Click **Start Benchmark Run** to watch live thoughts, tool actions, and verifications.
4. If you are satisfied with the results, click **Publish to Git** right from the UI!

#### Option B: Headless CLI
```bash
# Run Redis benchmark
cargo run --release -- run --task redis --model openrouter/meta/muse-spark-1.3-contributor --effort low

# Run HTTP benchmark
cargo run --release -- run --task http --model openrouter/meta/muse-spark-1.3-contributor --effort low
```

### 4. Publishing & Contributing to This Repository
When your benchmark completes and you approve the results:
```bash
# 1. Commit run, snapshot environment info, and update SUMMARY.md:
cargo run --release -- publish --run-id <run_id>

# 2. Push to your branch and submit a Pull Request:
git push origin my-benchmark-results
```
Your run directory (`runs/<run_id>/`) contains:
- `workspace/`: The complete candidate codebase produced by the model.
- `manifest.json`: Full metrics, token consumption, live spend, and test stages.
- `console.log`: Complete audit trail of thoughts, bash commands, and test outputs.
- `env.json`: Hardware, operating system, and container specifications.
- `README.md`: Self-contained markdown report for that specific run.

---
*Maintained by the SubDollarBench Community. Under-$1 LLMs can build real software.* 🚀
