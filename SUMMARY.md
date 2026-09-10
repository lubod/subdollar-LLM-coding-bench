# 🏆 SubDollarBench: Autonomous Under-$1 LLM Systems Benchmark

> **Measuring under-$1/1M token LLMs on autonomous, production-grade SWE systems engineering tasks (Redis & HTTP/1.1) in isolated Docker sandboxes.**

*Last Updated: 2026-09-10 07:45:28 UTC*

## 📑 Quick Navigation
- [📊 Global Leaderboard](#-global-leaderboard)
- [⚡ Task 1: In-Memory Redis Server](#-task-1-in-memory-redis-server-task-redis)
- [🌐 Task 2: HTTP/1.1 Web Server](#-task-2-http11-web-server-task-http)
- [🖥️ Testbed Environment Specs](#%EF%B8%8F-benchmark-testbed-environment-specs)
- [🚀 How to Reproduce & Submit Results](#-how-to-reproduce--submit-your-results)

---

## 📊 Global Leaderboard

| Rank | Model | Effort | Task | Lang | Pass Rate | Duration | Turns | Throughput | Cost (USD) | Efficiency | Full Trace & Code |
|:---:|:---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| 🥇 1 | `openrouter/meta/muse-spark-1.3-contributor` | `auto` | HTTP/1.1 | `Python` | 100% (5/5) | 1m 18s | 9 | 30976 req/s *(93% ref)* | $0.0027 *(eff: $0.0130)* | **220.8** pts/¢ *(base: 77.1)* | [Inspect](runs/http_openrouter_meta_muse-spark-1.3-contributor_20260907_125627/) |
| 🥈 2 | `openai/gpt-5.6-luna-pro` | `auto` | HTTP/1.1 | `Python` | 100% (5/5) | 1m 29s | 4 | 5853 req/s *(18% ref)* | $0.0167 *(eff: $0.0279)* | **48.4** pts/¢ *(base: 35.8)* | [Inspect](runs/http_openai_gpt-5.6-luna-pro_20260907_160545/) |
| 🥉 3 | `openrouter/meta/muse-spark-1.3-contributor` | `auto` | Redis | `Go` | 100% (4/4) | 1m 48s | 27 | N/A | $0.0042 *(eff: $0.0345)* | **28.9** pts/¢ | [Inspect](runs/redis_openrouter_meta_muse-spark-1.3-contributor_20260909_180249/) |
| 4 | `openai/gpt-5.6-luna-pro` | `auto` | HTTP/1.1 | `Python` | 100% (5/5) | 2m 1s | 9 | 5696 req/s *(17% ref)* | $0.0223 *(eff: $0.0463)* | **29.0** pts/¢ *(base: 21.6)* | [Inspect](runs/http_openai_gpt-5.6-luna-pro_20260909_155732/) |
| 5 | `openrouter/google/gemini-3.8-flash` | `auto` | HTTP/1.1 | `Go` | 100% (5/5) | 1m 36s | 9 | 46151 req/s *(139% ref)* | $0.0651 *(eff: $0.0740)* | **51.0** pts/¢ *(base: 13.5)* | [Inspect](runs/http_openrouter_google_gemini-3.8-flash_20260909_163707/) |
| 6 | `openrouter/tencent/hy4-preview` | `auto` | HTTP/1.1 | `Python` | 100% (5/5) | 6m 22s | 33 | 14802 req/s *(47% ref)* | $0.1283 *(eff: $0.1809)* | **10.7** pts/¢ *(base: 5.5)* | [Inspect](runs/http_openrouter_tencent_hy4-preview_20260909_174509/) |
| 7 | `openrouter/~deepseek/deepseek-v4-flash-latest` | `auto` | HTTP/1.1 | `Python` | 40% (2/5) | 5m 44s | 15 | N/A | $0.0153 *(eff: $0.0388)* | **0.0** pts/¢ | [Inspect](runs/http_openrouter_~deepseek_deepseek-v4-flash-latest_20260907_142657/) |
| 8 | `openai/gpt-5.6-luna` | `auto` | HTTP/1.1 | `Unknown` | 0% (0/5) | 3s | 48 | N/A | $0.0000 | **0.0** pts/¢ | [Inspect](runs/http_openai_gpt-5.6-luna_20260907_145111/) |
| 9 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Unknown` | 0% (0/5) | 20s | 1 | N/A | $0.0000 *(eff: $0.0011)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_072433/) |
| 10 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Unknown` | 0% (0/5) | 57s | 1 | N/A | $0.0000 *(eff: $0.0022)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_072100/) |
| 11 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Unknown` | 0% (0/5) | 58s | 1 | N/A | $0.0000 *(eff: $0.0022)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_081009/) |
| 12 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Python` | 0% (0/5) | 1m 30s | 3 | N/A | $0.0000 *(eff: $0.0043)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_114159/) |
| 13 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Python` | 0% (0/5) | 2m 18s | - | N/A | $0.0000 *(eff: $0.0044)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_084014/) |
| 14 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Go` | 0% (0/5) | 5m 9s | 2 | N/A | $0.0000 *(eff: $0.0104)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_092044/) |
| 15 | `openai/qwen2.5-coder-7b` | `auto` | HTTP/1.1 | `Unknown` | 0% (0/5) | 6m 37s | 1 | N/A | $0.0000 *(eff: $0.0125)* | **0.0** pts/¢ | [Inspect](runs/http_openai_qwen2.5-coder-7b_20260907_075504/) |
| 16 | `openrouter/~z-ai/glm-flash-latest` | `auto` | HTTP/1.1 | `Unknown` | 0% (0/5) | 15m 4s | - | N/A | $0.0000 *(eff: $0.0271)* | **0.0** pts/¢ | [Inspect](runs/http_openrouter_~z-ai_glm-flash-latest_20260907_125946/) |
| 17 | `openrouter/z-ai/glm-5.3-flash` | `auto` | HTTP/1.1 | `Go` | 0% (0/5) | 6m 20s | 15 | N/A | $0.0152 *(eff: $0.0413)* | **0.0** pts/¢ | [Inspect](runs/http_openrouter_z-ai_glm-5.3-flash_20260909_160518/) |

---

## ⚡ Task 1: In-Memory Redis Server (`task: redis`)

The candidate LLM is instructed to build a production-grade, concurrent Redis clone from scratch.
- **Wire Protocol**: Full RESP2 parser handling pipelined TCP streams, bulk strings, integers, simple strings, and errors.
- **Supported Commands**: `PING`, `ECHO`, `SET` (with `PX` millisecond expiration, `EX`, `NX`, `XX`), `GET`, `DEL`, `EXISTS`, `INCR`, `DECR`, `COMMAND`, `QUIT`.
- **Packaging Freedom**: Working multi-stage `Dockerfile` (automatically built and containerized) or `./start.sh`.
- **Verification**: Automated raw-socket TCP verification test suite + `redis-benchmark -p 6379 -n 5000 -c 10 -t get,set -q`.

| Model | Effort | Lang | Pass Rate | Duration | Turns | Throughput | Cost | Score | Run Archive |
|:---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `openrouter/meta/muse-spark-1.3-contributor` | `auto` | `Go` | 100% | 1m 48s | 27 | N/A | $0.0042 | 28.9 pts/¢ | [runs/redis_openrouter_meta_muse-spark-1.3-contributor_20260909_180249/](runs/redis_openrouter_meta_muse-spark-1.3-contributor_20260909_180249/) |

---

## 🌐 Task 2: HTTP/1.1 Web Server (`task: http`)

The candidate LLM is instructed to build an RFC 7230/7231 HTTP/1.1 web server from scratch.
- **Features**: `GET /` (200 OK), `404 Not Found Handling`, `GET /echo/{str}` with dynamic `Content-Length`, `GET /user-agent` header reflection, `POST` and `GET /files/{filename}` file persistence, HTTP/1.1 socket keep-alive reuse.
- **Packaging Freedom**: Working `Dockerfile` or executable `./start.sh`.
- **Verification**: 5-stage automated conformance suite + `wrk -t2 -c20 -d3s http://127.0.0.1:8080/`.

| Model | Effort | Lang | Pass Rate | Duration | Turns | Throughput | Cost | Score | Run Archive |
|:---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `openrouter/meta/muse-spark-1.3-contributor` | `auto` | `Python` | 100% | 1m 18s | 9 | 30976 req/s | $0.0027 | 77.1 pts/¢ | [runs/http_openrouter_meta_muse-spark-1.3-contributor_20260907_125627/](runs/http_openrouter_meta_muse-spark-1.3-contributor_20260907_125627/) |
| `openai/gpt-5.6-luna-pro` | `auto` | `Python` | 100% | 1m 29s | 4 | 5853 req/s | $0.0167 | 35.8 pts/¢ | [runs/http_openai_gpt-5.6-luna-pro_20260907_160545/](runs/http_openai_gpt-5.6-luna-pro_20260907_160545/) |
| `openai/gpt-5.6-luna-pro` | `auto` | `Python` | 100% | 2m 1s | 9 | 5696 req/s | $0.0223 | 21.6 pts/¢ | [runs/http_openai_gpt-5.6-luna-pro_20260909_155732/](runs/http_openai_gpt-5.6-luna-pro_20260909_155732/) |
| `openrouter/google/gemini-3.8-flash` | `auto` | `Go` | 100% | 1m 36s | 9 | 46151 req/s | $0.0651 | 13.5 pts/¢ | [runs/http_openrouter_google_gemini-3.8-flash_20260909_163707/](runs/http_openrouter_google_gemini-3.8-flash_20260909_163707/) |
| `openrouter/tencent/hy4-preview` | `auto` | `Python` | 100% | 6m 22s | 33 | 14802 req/s | $0.1283 | 5.5 pts/¢ | [runs/http_openrouter_tencent_hy4-preview_20260909_174509/](runs/http_openrouter_tencent_hy4-preview_20260909_174509/) |
| `openrouter/~deepseek/deepseek-v4-flash-latest` | `auto` | `Python` | 40% | 5m 44s | 15 | N/A | $0.0153 | 0.0 pts/¢ | [runs/http_openrouter_~deepseek_deepseek-v4-flash-latest_20260907_142657/](runs/http_openrouter_~deepseek_deepseek-v4-flash-latest_20260907_142657/) |
| `openai/gpt-5.6-luna` | `auto` | `Unknown` | 0% | 3s | 48 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_gpt-5.6-luna_20260907_145111/](runs/http_openai_gpt-5.6-luna_20260907_145111/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Unknown` | 0% | 20s | 1 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_072433/](runs/http_openai_qwen2.5-coder-7b_20260907_072433/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Unknown` | 0% | 57s | 1 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_072100/](runs/http_openai_qwen2.5-coder-7b_20260907_072100/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Unknown` | 0% | 58s | 1 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_081009/](runs/http_openai_qwen2.5-coder-7b_20260907_081009/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Python` | 0% | 1m 30s | 3 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_114159/](runs/http_openai_qwen2.5-coder-7b_20260907_114159/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Python` | 0% | 2m 18s | - | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_084014/](runs/http_openai_qwen2.5-coder-7b_20260907_084014/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Go` | 0% | 5m 9s | 2 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_092044/](runs/http_openai_qwen2.5-coder-7b_20260907_092044/) |
| `openai/qwen2.5-coder-7b` | `auto` | `Unknown` | 0% | 6m 37s | 1 | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openai_qwen2.5-coder-7b_20260907_075504/](runs/http_openai_qwen2.5-coder-7b_20260907_075504/) |
| `openrouter/~z-ai/glm-flash-latest` | `auto` | `Unknown` | 0% | 15m 4s | - | N/A | $0.0000 | 0.0 pts/¢ | [runs/http_openrouter_~z-ai_glm-flash-latest_20260907_125946/](runs/http_openrouter_~z-ai_glm-flash-latest_20260907_125946/) |
| `openrouter/z-ai/glm-5.3-flash` | `auto` | `Go` | 0% | 6m 20s | 15 | N/A | $0.0152 | 0.0 pts/¢ | [runs/http_openrouter_z-ai_glm-5.3-flash_20260909_160518/](runs/http_openrouter_z-ai_glm-5.3-flash_20260909_160518/) |

---

## 📡 Task 3: DNS Server (`task: dns`)

The candidate LLM is instructed to build an RFC 1035 UDP DNS Server from scratch.
- **Wire Protocol**: Raw UDP query resolver handling RFC 1035 packet headers, question queries, and A-record resolution.
- **Supported Queries**: A record lookups, standard query flags (QR, Opcode, AA, RD, RA, RCODE), dynamic port listening.
- **Packaging Freedom**: Working multi-stage `Dockerfile` (automatically built and containerized) or `./start.sh`.
- **Verification**: 4-stage automated UDP conformance suite + `queryperf` load test.

---

## 🖥️ Benchmark Testbed Environment Specs

All runs recorded in this repository were benchmarked under identical, isolated hardware and sandbox conditions:

| Component | Specification |
|:---|:---|
| **Operating System** | Ubuntu 24.04.3 LTS |
| **Kernel & Architecture** | Linux 7.0.0-31-generic (x86_64) |
| **CPU Model** | AMD Ryzen AI 9 HX 370 w/ Radeon 890M (24 vCPUs) |
| **System Memory** | 54.5 GiB |
| **Docker Runtime** | Docker version 29.1.3, build 29.1.3-0ubuntu3~24.04.2 |
| **Rust Version** | N/A |
| **Python Version** | Python 3.12.3 |
| **Node.js Version** | v24.13.0 |
| **Git Baseline** | Commit `1ed9025` (branch `master`) |
| **Benchmark Captured** | 2026-09-10T07:45:28.384007912+00:00 |

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
