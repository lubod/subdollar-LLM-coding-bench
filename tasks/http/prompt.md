You are building an RFC-compliant HTTP/1.1 web server from scratch.

### Constraints & Language Freedom
- You are free to choose ANY programming language (Go, Python, Rust, Node.js, C, etc.).
- All code must reside in the current workspace.
- You MUST create an executable `./start.sh` script that starts your server listening on `0.0.0.0:8080`.

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

Test your server thoroughly before finishing!
