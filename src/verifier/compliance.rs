use std::fs;
use std::path::Path;

pub struct ComplianceChecker;

const FORBIDDEN_BASE_IMAGES: &[&str] = &[
    "redis", "valkey", "keydb", "dragonfly", "nginx", "caddy", "coredns",
    "bind9", "named", "dnsmasq", "apache", "httpd", "envoy", "traefik",
    "haproxy", "lighttpd", "memcached",
];

const FORBIDDEN_DAEMONS: &[&str] = &[
    "redis-server", "valkey-server", "keydb-server", "dragonfly", "nginx", "caddy",
    "coredns", "dnsmasq", "named", "bind9", "lighttpd", "apache2", "httpd", "memcached",
];

impl ComplianceChecker {
    /// Scans the candidate workspace to ensure no prohibited server frameworks, prebuilt server daemons,
    /// or built-in high-level protocol modules are used.
    pub fn check_no_frameworks(workdir: &Path) -> Result<(), String> {
        let mut violations = Vec::new();
        Self::scan_dir(workdir, &mut violations);

        if violations.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Anti-Cheat Violation: Found forbidden framework/daemon usage: {}",
                violations.join("; ")
            ))
        }
    }

    fn scan_dir(dir: &Path, violations: &mut Vec<String>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden folders, git, target, node_modules, etc.
                if name.starts_with('.') || name == "target" || name == "node_modules" || name == "vendor" || name == "__pycache__" {
                    continue;
                }

                if path.is_dir() {
                    Self::scan_dir(&path, violations);
                } else if path.is_file() {
                    Self::inspect_file(&path, &name, violations);
                }
            }
        }
    }

    /// Normalizes lines by concatenating trailing `\` line continuations into unified logical lines.
    pub fn normalize_lines(content: &str) -> Vec<String> {
        let mut logical_lines = Vec::new();
        let mut current = String::new();

        for raw_line in content.lines() {
            let trimmed = raw_line.trim();
            if current.is_empty() && trimmed.starts_with('#') {
                logical_lines.push(trimmed.to_string());
                continue;
            }

            if let Some(stripped) = trimmed.strip_suffix('\\') {
                current.push_str(stripped.trim_end());
                current.push(' ');
            } else {
                current.push_str(trimmed);
                if !current.trim().is_empty() {
                    logical_lines.push(current.trim().to_string());
                }
                current.clear();
            }
        }
        if !current.trim().is_empty() {
            logical_lines.push(current.trim().to_string());
        }
        logical_lines
    }

    /// Tokenizes shell or Dockerfile commands by splitting on shell word/operator delimiters.
    pub fn tokenize_command(cmd: &str) -> Vec<String> {
        let cleaned: String = cmd
            .chars()
            .map(|c| match c {
                '[' | ']' | '(' | ')' | '{' | '}' | '"' | '\'' | '`' | '$' | ';' | '&' | '|'
                | ',' | '=' | '!' | '<' | '>' | '\t' => ' ',
                _ => c,
            })
            .collect();
        cleaned.split_whitespace().map(|s| s.to_string()).collect()
    }

    fn inspect_file(path: &Path, filename: &str, violations: &mut Vec<String>) {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return, // Ignore binary or unreadable files
        };

        // 1. Dockerfile base-image and RUN/CMD/ENTRYPOINT daemon inspection
        if filename.eq_ignore_ascii_case("Dockerfile") || filename.contains("Dockerfile") {
            let logical_lines = Self::normalize_lines(&content);
            for line in logical_lines {
                let lower = line.to_lowercase();
                if lower.starts_with("from ") {
                    for forbidden in FORBIDDEN_BASE_IMAGES {
                        if lower.contains(forbidden) {
                            violations.push(format!("Dockerfile uses forbidden base image '{}'", line));
                            break;
                        }
                    }
                } else if lower.starts_with("run ")
                    || lower.starts_with("cmd ")
                    || lower.starts_with("cmd[")
                    || lower.starts_with("entrypoint ")
                    || lower.starts_with("entrypoint[")
                {
                    let tokens = Self::tokenize_command(&lower);
                    for daemon in FORBIDDEN_DAEMONS {
                        if tokens.iter().any(|t| t == daemon || t.ends_with(&format!("/{}", daemon))) {
                            violations.push(format!("Dockerfile: executes or installs forbidden server daemon '{}' in '{}'", daemon, line));
                            break;
                        }
                    }
                }
            }
        }

        // 2. Shell script daemon execution inspection
        if filename.ends_with(".sh") || filename.ends_with(".bash") || filename == "start.sh" {
            let logical_lines = Self::normalize_lines(&content);
            for line in logical_lines {
                if line.starts_with('#') {
                    continue;
                }
                let lower = line.to_lowercase();
                let tokens = Self::tokenize_command(&lower);
                for daemon in FORBIDDEN_DAEMONS {
                    if tokens.iter().any(|t| t == daemon || t.ends_with(&format!("/{}", daemon))) {
                        violations.push(format!("{}: executes forbidden server daemon '{}'", filename, daemon));
                        break;
                    }
                }
            }
        }

        // 3. Node.js package.json
        if filename == "package.json" {
            let forbidden = [
                "\"express\"", "\"fastify\"", "\"koa\"", "\"hapi\"", "\"nest\"",
                "\"sails\"", "\"connect\"", "\"ioredis\"", "\"redis\"",
            ];
            for f in forbidden {
                if content.contains(f) {
                    violations.push(format!("package.json: uses forbidden dependency {}", f));
                }
            }
        }

        // 4. Python requirements.txt or pyproject.toml
        if filename == "requirements.txt" || filename == "pyproject.toml" {
            let forbidden = [
                "flask", "fastapi", "django", "tornado", "aiohttp", "sanic",
                "redis", "valkey", "redis-py", "twisted",
            ];
            for line in content.lines() {
                let trimmed = line.trim().to_lowercase();
                for f in forbidden {
                    if trimmed.starts_with(f) || trimmed.contains(&format!("\"{}\"", f)) || trimmed.contains(&format!("'{}'", f)) {
                        violations.push(format!("{}: uses forbidden dependency '{}'", filename, f));
                    }
                }
            }
        }

        // 5. Rust Cargo.toml
        if filename == "Cargo.toml" {
            let forbidden = [
                "actix-web", "axum", "warp", "rocket", "tide", "hyper",
                "redis", "fred",
            ];
            for f in forbidden {
                if content.contains(f) {
                    violations.push(format!("Cargo.toml: uses forbidden dependency '{}'", f));
                }
            }
        }

        // 6. Go go.mod
        if filename == "go.mod" {
            let forbidden = [
                "github.com/gin-gonic/gin", "github.com/labstack/echo",
                "github.com/gofiber/fiber", "github.com/go-redis/redis",
                "github.com/redis/go-redis",
            ];
            for f in forbidden {
                if content.contains(f) {
                    violations.push(format!("go.mod: uses forbidden dependency '{}'", f));
                }
            }
        }

        // 7. Source Code inspection (Python, Go, JS, TS, Rust, C)
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "py" => {
                if content.contains("http.server") {
                    violations.push(format!("{}: imports built-in http.server", filename));
                }
                if content.contains("import flask") || content.contains("from flask") {
                    violations.push(format!("{}: imports flask", filename));
                }
                if content.contains("import fastapi") || content.contains("from fastapi") {
                    violations.push(format!("{}: imports fastapi", filename));
                }
            }
            "go" => {
                if content.contains("http.ListenAndServe") || content.contains("http.Server{") {
                    violations.push(format!("{}: calls net/http built-in server (http.ListenAndServe)", filename));
                }
            }
            "js" | "ts" => {
                if content.contains("http.createServer") {
                    violations.push(format!("{}: calls node:http built-in server (http.createServer)", filename));
                }
                if content.contains("require('express')") || content.contains("from 'express'") {
                    violations.push(format!("{}: imports express", filename));
                }
            }
            "rs" => {
                if content.contains("actix_web::") || content.contains("axum::") {
                    violations.push(format!("{}: references web framework", filename));
                }
            }
            "c" | "cpp" | "cc" | "h" | "hpp" => {
                if content.contains("<microhttpd.h>") {
                    violations.push(format!("{}: includes forbidden library <microhttpd.h>", filename));
                }
                if content.contains("<event2/http.h>") {
                    violations.push(format!("{}: includes forbidden library <event2/http.h>", filename));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compliance_checker_clean_workspace() {
        let temp_dir = std::env::temp_dir().join(format!("test_compliance_clean_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        fs::write(
            temp_dir.join("main.go"),
            "package main\nimport \"net\"\nfunc main() { listener, _ := net.Listen(\"tcp\", \":8080\") }\n",
        ).unwrap();
        fs::write(temp_dir.join("Dockerfile"), "FROM golang:1.22-alpine\nCOPY . /app\n").unwrap();
        fs::write(temp_dir.join("start.sh"), "#!/bin/bash\n./server\n").unwrap();

        assert!(ComplianceChecker::check_no_frameworks(&temp_dir).is_ok());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compliance_checker_catches_continuation_line_and_delimiters() {
        let temp_dir = std::env::temp_dir().join(format!("test_compliance_cont_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Dockerfile with line continuations
        fs::write(
            temp_dir.join("Dockerfile"),
            "FROM ubuntu:24.04\nRUN apt-get update && \\\n    apt-get install -y \\\n    redis-server\n",
        ).unwrap();
        let res1 = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res1.is_err());
        assert!(res1.unwrap_err().contains("redis-server"));
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 2. Dockerfile with continuation in FROM
        fs::write(
            temp_dir.join("Dockerfile"),
            "FROM \\\n  redis:alpine\n",
        ).unwrap();
        let res2 = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res2.is_err());
        assert!(res2.unwrap_err().contains("redis"));
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 3. Shell script with delimiter variants (;, &, quotes, $())
        let bad_scripts = [
            "#!/bin/bash\nredis-server;\n",
            "#!/bin/bash\nredis-server&\n",
            "#!/bin/bash\n\"redis-server\"\n",
            "#!/bin/bash\n$(redis-server)\n",
            "#!/bin/bash\n`redis-server`\n",
            "#!/bin/bash\necho starting && \\\n  redis-server\n",
        ];
        for script in bad_scripts {
            fs::write(temp_dir.join("start.sh"), script).unwrap();
            let res = ComplianceChecker::check_no_frameworks(&temp_dir);
            assert!(res.is_err(), "Failed to catch script: {}", script);
            assert!(res.unwrap_err().contains("redis-server"));
            let _ = fs::remove_file(temp_dir.join("start.sh"));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compliance_checker_catches_forbidden_frameworks() {
        let temp_dir = std::env::temp_dir().join(format!("test_compliance_fail_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Python http.server
        fs::write(temp_dir.join("server.py"), "from http.server import HTTPServer\n").unwrap();
        let res_py = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_py.is_err());
        assert!(res_py.unwrap_err().contains("http.server"));
        let _ = fs::remove_file(temp_dir.join("server.py"));

        // 2. Node express in package.json
        fs::write(temp_dir.join("package.json"), "{\"dependencies\": {\"express\": \"^4.18\"}}\n").unwrap();
        let res_pkg = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_pkg.is_err());
        assert!(res_pkg.unwrap_err().contains("package.json"));
        let _ = fs::remove_file(temp_dir.join("package.json"));

        // 3. Go net/http.ListenAndServe
        fs::write(temp_dir.join("main.go"), "package main\nfunc main() { http.ListenAndServe(\":8080\", nil) }\n").unwrap();
        let res_go = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_go.is_err());
        assert!(res_go.unwrap_err().contains("net/http"));
        let _ = fs::remove_file(temp_dir.join("main.go"));

        // 4. Rust Cargo.toml axum
        fs::write(temp_dir.join("Cargo.toml"), "[dependencies]\naxum = \"0.7\"\n").unwrap();
        let res_rs = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_rs.is_err());
        assert!(res_rs.unwrap_err().contains("Cargo.toml"));
        let _ = fs::remove_file(temp_dir.join("Cargo.toml"));

        // 5. Python requirements.txt
        fs::write(temp_dir.join("requirements.txt"), "flask==3.0.0\n").unwrap();
        let res_req = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_req.is_err());
        assert!(res_req.unwrap_err().contains("requirements.txt"));
        let _ = fs::remove_file(temp_dir.join("requirements.txt"));

        // 6. Go go.mod
        fs::write(temp_dir.join("go.mod"), "module myapp\nrequire github.com/gin-gonic/gin v1.9.1\n").unwrap();
        let res_mod = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_mod.is_err());
        assert!(res_mod.unwrap_err().contains("go.mod"));
        let _ = fs::remove_file(temp_dir.join("go.mod"));

        // 7. Node http.createServer
        fs::write(temp_dir.join("server.js"), "const http = require('http'); http.createServer();\n").unwrap();
        let res_js = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_js.is_err());
        assert!(res_js.unwrap_err().contains("http.createServer"));
        let _ = fs::remove_file(temp_dir.join("server.js"));

        // 8. Python flask / fastapi imports
        fs::write(temp_dir.join("app.py"), "from flask import Flask\nfrom fastapi import FastAPI\n").unwrap();
        let res_app = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_app.is_err());
        assert!(res_app.unwrap_err().contains("flask"));
        let _ = fs::remove_file(temp_dir.join("app.py"));

        // 9. Dockerfile FROM redis cheat
        fs::write(temp_dir.join("Dockerfile"), "FROM redis:alpine\nEXPOSE 6379\n").unwrap();
        let res_df = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_df.is_err());
        assert!(res_df.unwrap_err().contains("redis"));
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 10. start.sh redis-server cheat
        fs::write(temp_dir.join("start.sh"), "#!/bin/bash\nredis-server --port 6379\n").unwrap();
        let res_sh = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_sh.is_err());
        assert!(res_sh.unwrap_err().contains("redis-server"));
        let _ = fs::remove_file(temp_dir.join("start.sh"));

        // 11. C/C++ microhttpd
        fs::write(temp_dir.join("server.c"), "#include <microhttpd.h>\n").unwrap();
        let res_c = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_c.is_err());
        assert!(res_c.unwrap_err().contains("microhttpd"));
        let _ = fs::remove_file(temp_dir.join("server.c"));

        // 12. Dockerfile RUN redis-server install
        fs::write(temp_dir.join("Dockerfile"), "FROM ubuntu:24.04\nRUN apt-get update && apt-get install -y redis-server\n").unwrap();
        let res_df_run = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_df_run.is_err());
        assert!(res_df_run.unwrap_err().contains("redis-server"));
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 13. Dockerfile CMD redis-server
        fs::write(temp_dir.join("Dockerfile"), "FROM ubuntu:24.04\nCMD [\"redis-server\", \"--protected-mode\", \"no\"]\n").unwrap();
        let res_df_cmd = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_df_cmd.is_err());
        assert!(res_df_cmd.unwrap_err().contains("redis-server"));
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 14. Dockerfile ENTRYPOINT nginx
        fs::write(temp_dir.join("Dockerfile"), "FROM alpine:3.19\nENTRYPOINT [\"nginx\", \"-g\", \"daemon off;\"]\n").unwrap();
        let res_df_ep = ComplianceChecker::check_no_frameworks(&temp_dir);
        assert!(res_df_ep.is_err());
        assert!(res_df_ep.unwrap_err().contains("nginx"));
        let _ = fs::remove_file(temp_dir.join("Dockerfile"));

        // 15. Hidden folder ignore
        let hidden = temp_dir.join(".git");
        let _ = fs::create_dir_all(&hidden);
        fs::write(hidden.join("server.py"), "import http.server\n").unwrap();
        assert!(ComplianceChecker::check_no_frameworks(&temp_dir).is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
