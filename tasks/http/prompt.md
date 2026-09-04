You are building an RFC-compliant HTTP/1.1 web server from scratch.

### Software Engineering (SWE) & Code Quality Standards
Your implementation must meet high professional software engineering standards:
1. **Clean Architecture & Modularity**: Structure code with clear separation of concerns (TCP listener, HTTP request/header parser, router/handler mapping, file I/O layer, and response formatter). Avoid monolithic scripts or hardcoded shortcuts.
2. **Robust Concurrency & Thread Safety**: Ensure the server can handle multiple concurrent HTTP client requests without race conditions, deadlocks, or socket blocking.
3. **Resilient Error Handling**: Safely handle malformed HTTP requests, missing headers, large payloads, file permission errors, non-existent paths, and sudden client disconnects without crashing or leaking file descriptors.
4. **Idiomatic Style & Best Practices**: Write clean, idiomatic code adhering to standard conventions for your chosen language (e.g., Go, Rust, Python, Node.js).
5. **Production Readiness**: Code should be production-grade, maintainable, performant, and robust under load.

### CRITICAL INTEGRITY RULE: NO ACCESSING OR CONSULTING SOURCE CODE
- **Strict Prohibition**: You are strictly FORBIDDEN from downloading, curling, scraping, cloning, reading, or consulting the source code of the reference implementation or any external implementation (e.g., Nginx, Apache, or external web server source repositories).
- **Allowed Resources**: You may consult official documentation, API specifications, and official RFCs (e.g., RFC 7230, RFC 7231).
- **Black-Box Testing**: You may test runtime behavior against the running reference server using CLI tools (`curl -v http://localhost:8081/...`).
- **Originality**: All code must be your own original implementation designed from specifications and observable behavior. Any access to reference source code invalidates the benchmark run.

### Constraints & Packaging Freedom (Dockerfile or start.sh)
- You are free to choose ANY programming language (Go, Python, Rust, Node.js, C, etc.).
- All code and configuration must reside in the current workspace.
- **Entrypoint / Execution Options (You can choose either)**:
  1. **Working Dockerfile (Recommended)**: Create a working `Dockerfile` that builds and packages your server and exposes port 8080. If a `Dockerfile` is present in the workspace, the test harness will automatically build and run your Docker container directly!
  2. **Executable `./start.sh`**: Alternatively, create an executable `./start.sh` script that launches your server listening on `0.0.0.0:8080`. Our pre-configured test sandbox has Rust (cargo), Go, Python 3, Node.js, and C/C++ pre-installed.
  (Both options are fully supported by the benchmark runner!)

### Reference Server Available for Testing
An official reference web server is running on port 8081.
You can use `curl -v http://localhost:8081/...` to inspect standard behavior.
You can test your own server with `curl -v http://localhost:8080/...`.

### Required Endpoints & Behavior
Your server must listen on port 8080 and implement HTTP/1.1:

1. **Root Endpoint**:
   - `GET /` -> responds with `HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n`
2. **404 Handling**:
   - Any unknown path (e.g. `GET /random-path`) -> responds with `HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n`
3. **Echo Endpoint**:
   - `GET /echo/{str}` -> responds with `200 OK`, `Content-Type: text/plain`, and the body containing `{str}` with exact `Content-Length`.
4. **User-Agent Header**:
   - `GET /user-agent` -> reads the incoming `User-Agent` header and echoes its value back in the body with `Content-Type: text/plain`.
5. **File Storage**:
   - Ensure the directory `/tmp/files` exists.
   - `POST /files/{filename}` -> reads request body and writes it to `/tmp/files/{filename}`. Responds with `201 Created`.
   - `GET /files/{filename}` -> reads file `/tmp/files/{filename}`. If found, responds with `200 OK`, `Content-Type: application/octet-stream`, `Content-Length: <size>`, and the file bytes. If not found, responds with `404 Not Found`.

### Verification Criteria
1. The server will be verified with a test suite exercising all status codes, headers, and file uploads/downloads.
2. The server will be stress-tested with `wrk -t2 -c20 -d3s http://localhost:8080/`.

Test your server thoroughly with `curl` before finishing!
