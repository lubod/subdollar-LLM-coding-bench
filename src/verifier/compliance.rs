use std::fs;
use std::path::Path;

pub struct ComplianceChecker;

impl ComplianceChecker {
    /// Scans the candidate workspace to ensure no prohibited HTTP frameworks or built-in HTTP server modules are used.
    pub fn check_no_frameworks(workdir: &Path) -> Result<(), String> {
        let mut violations = Vec::new();
        Self::scan_dir(workdir, &mut violations);

        if violations.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Anti-Cheat Violation: Found forbidden HTTP framework/module usage: {}",
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

    fn inspect_file(path: &Path, filename: &str, violations: &mut Vec<String>) {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return, // Ignore binary or unreadable files
        };

        // 1. Node.js package.json
        if filename == "package.json" {
            let forbidden = ["\"express\"", "\"fastify\"", "\"koa\"", "\"hapi\"", "\"nest\"", "\"sails\"", "\"connect\""];
            for f in forbidden {
                if content.contains(f) {
                    violations.push(format!("package.json specifies forbidden dependency {}", f));
                }
            }
        }

        // 2. Python requirements.txt or pyproject.toml
        if filename == "requirements.txt" || filename == "pyproject.toml" || filename == "Pipfile" {
            let forbidden = ["flask", "fastapi", "django", "tornado", "sanic", "starlette", "bottle", "aiohttp", "gunicorn", "uvicorn"];
            for line in content.lines() {
                let lower = line.to_lowercase();
                for f in forbidden {
                    if lower.starts_with(f) || lower.contains(&format!("\"{}\"", f)) {
                        violations.push(format!("{} specifies forbidden dependency {}", filename, f));
                    }
                }
            }
        }

        // 3. Go go.mod
        if filename == "go.mod" {
            let forbidden = [
                "github.com/gin-gonic/gin",
                "github.com/gofiber/fiber",
                "github.com/labstack/echo",
                "github.com/gorilla/mux",
                "github.com/go-chi/chi",
            ];
            for f in forbidden {
                if content.contains(f) {
                    violations.push(format!("go.mod specifies forbidden dependency {}", f));
                }
            }
        }

        // 4. Rust Cargo.toml
        if filename == "Cargo.toml" {
            let forbidden = ["axum", "actix-web", "warp", "rocket", "tide", "poem", "salvo"];
            for line in content.lines() {
                let trimmed = line.trim();
                for f in forbidden {
                    if trimmed.starts_with(f) || trimmed.starts_with(&format!("\"{}\"", f)) {
                        violations.push(format!("Cargo.toml specifies forbidden dependency {}", f));
                    }
                }
            }
        }

        // 5. Source code inspection (.py, .go, .rs, .js, .ts)
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        match ext {
            "py" => {
                if content.contains("import http.server") || content.contains("from http.server") {
                    violations.push(format!("{}: imports built-in http.server", filename));
                }
                if content.contains("from flask import") || content.contains("import flask") {
                    violations.push(format!("{}: imports flask", filename));
                }
                if content.contains("from fastapi import") || content.contains("import fastapi") {
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

        assert!(ComplianceChecker::check_no_frameworks(&temp_dir).is_ok());
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

        // 9. Hidden folder ignore
        let hidden = temp_dir.join(".git");
        let _ = fs::create_dir_all(&hidden);
        fs::write(hidden.join("server.py"), "import http.server\n").unwrap();
        assert!(ComplianceChecker::check_no_frameworks(&temp_dir).is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
