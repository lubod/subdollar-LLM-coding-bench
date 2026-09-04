use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::verifier::StageResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTokenUsage {
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileInfo {
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub model: String,
    pub task: String,
    pub status: String,
    pub language: String,
    pub started_at: String,
    pub completed_at: String,
    pub duration_seconds: f64,
    pub pass_rate: f64,
    pub passed_stages: u32,
    pub total_stages: u32,
    pub stages: Vec<StageResult>,
    pub throughput_req_sec: Option<f64>,
    pub tokens: RunTokenUsage,
    pub cost_usd: f64,
    pub savings_percent: f64,
    pub efficiency_score: f64,
    pub files: Vec<FileInfo>,
}

pub struct RunArchiver;

impl RunArchiver {
    pub fn resolve_runs_dir() -> PathBuf {
        if Path::new("./runs").exists() {
            PathBuf::from("./runs")
        } else if Path::new("/home/ubuntu/subdollar-LLM-coding-bench/runs").exists() {
            PathBuf::from("/home/ubuntu/subdollar-LLM-coding-bench/runs")
        } else {
            PathBuf::from("./runs")
        }
    }

    pub fn archive_run(
        runs_dir: &Path,
        manifest: &RunManifest,
        workspace_dir: &Path,
        console_log: &str,
    ) -> Result<PathBuf> {
        let run_dir = runs_dir.join(&manifest.run_id);
        fs::create_dir_all(&run_dir)?;

        // 1. Write manifest.json
        let manifest_json = serde_json::to_string_pretty(manifest)?;
        fs::write(run_dir.join("manifest.json"), manifest_json)?;

        // 2. Write console.log
        fs::write(run_dir.join("console.log"), console_log)?;

        // 3. Snapshot candidate workspace files
        let ws_dest = run_dir.join("workspace");
        fs::create_dir_all(&ws_dest)?;
        if workspace_dir.exists() {
            Self::copy_dir_recursive(workspace_dir, &ws_dest)?;
        }

        Ok(run_dir)
    }

    pub fn list_runs(runs_dir: &Path) -> Vec<RunManifest> {
        let mut manifests = Vec::new();
        if let Ok(entries) = fs::read_dir(runs_dir) {
            for entry in entries.flatten() {
                let manifest_file = entry.path().join("manifest.json");
                if manifest_file.exists() {
                    if let Ok(content) = fs::read_to_string(&manifest_file) {
                        if let Ok(m) = serde_json::from_str::<RunManifest>(&content) {
                            manifests.push(m);
                        }
                    }
                }
            }
        }
        manifests.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        manifests
    }

    pub fn get_run(runs_dir: &Path, run_id: &str) -> Option<RunManifest> {
        let clean_id = Path::new(run_id).file_name()?.to_str()?;
        let manifest_file = runs_dir.join(clean_id).join("manifest.json");
        let content = fs::read_to_string(manifest_file).ok()?;
        serde_json::from_str::<RunManifest>(&content).ok()
    }

    pub fn get_console_log(runs_dir: &Path, run_id: &str) -> Option<String> {
        let clean_id = Path::new(run_id).file_name()?.to_str()?;
        let log_file = runs_dir.join(clean_id).join("console.log");
        fs::read_to_string(log_file).ok()
    }

    pub fn get_workspace_file(runs_dir: &Path, run_id: &str, filename: &str) -> Option<String> {
        let clean_id = Path::new(run_id).file_name()?.to_str()?;
        let clean_filename = Path::new(filename).file_name()?.to_str()?;
        let file_path = runs_dir.join(clean_id).join("workspace").join(clean_filename);
        fs::read_to_string(file_path).ok()
    }

    pub fn scan_workspace_files(workspace_dir: &Path) -> Vec<FileInfo> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(workspace_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if let Ok(meta) = entry.metadata() {
                        files.push(FileInfo {
                            name: p.file_name().unwrap_or_default().to_string_lossy().to_string(),
                            size_bytes: meta.len(),
                        });
                    }
                }
            }
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        files
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
        if !src.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let dest_path = dst.join(entry.file_name());
            if ft.is_dir() {
                let name = entry.file_name();
                let s = name.to_string_lossy();
                if s == ".git" || s == "target" || s == "node_modules" {
                    continue;
                }
                fs::create_dir_all(&dest_path)?;
                Self::copy_dir_recursive(&entry.path(), &dest_path)?;
            } else {
                fs::copy(entry.path(), dest_path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("test_archive_{}", std::process::id()));
        let runs_dir = temp_dir.join("runs");
        let ws_dir = temp_dir.join("workspace");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&ws_dir).unwrap();

        // Create sample workspace files
        fs::write(ws_dir.join("main.go"), "package main\nfunc main() {}\n").unwrap();
        fs::write(ws_dir.join("start.sh"), "#!/bin/bash\ngo run main.go\n").unwrap();

        let scanned = RunArchiver::scan_workspace_files(&ws_dir);
        assert_eq!(scanned.len(), 2);
        assert!(scanned.iter().any(|f| f.name == "main.go"));

        let manifest = RunManifest {
            run_id: "test_run_123".to_string(),
            model: "google/gemini-2.5-flash".to_string(),
            task: "redis".to_string(),
            status: "completed".to_string(),
            language: "Go".to_string(),
            started_at: "2026-09-04T06:30:00Z".to_string(),
            completed_at: "2026-09-04T06:31:00Z".to_string(),
            duration_seconds: 60.0,
            pass_rate: 100.0,
            passed_stages: 4,
            total_stages: 4,
            stages: vec![StageResult {
                stage: 1,
                name: "Stage 1: PING & ECHO".to_string(),
                passed: true,
                error: None,
            }],
            throughput_req_sec: Some(45000.0),
            tokens: RunTokenUsage {
                prompt_tokens: 10000,
                cached_tokens: 8000,
                completion_tokens: 1500,
                total_tokens: 11500,
            },
            cost_usd: 0.0025,
            savings_percent: 65.0,
            efficiency_score: 400.0,
            files: scanned,
        };

        let console_log = "[06:30:00] [INIT] Starting test run\n[06:31:00] [DONE] Finished!\n";
        let archived = RunArchiver::archive_run(&runs_dir, &manifest, &ws_dir, console_log).unwrap();
        assert!(archived.exists());

        // Verify listing
        let runs = RunArchiver::list_runs(&runs_dir);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "test_run_123");

        // Verify get_run
        let loaded = RunArchiver::get_run(&runs_dir, "test_run_123").unwrap();
        assert_eq!(loaded.model, "google/gemini-2.5-flash");
        assert_eq!(loaded.pass_rate, 100.0);

        // Verify get_console_log
        let log = RunArchiver::get_console_log(&runs_dir, "test_run_123").unwrap();
        assert!(log.contains("[INIT] Starting test run"));

        // Verify get_workspace_file
        let code = RunArchiver::get_workspace_file(&runs_dir, "test_run_123", "main.go").unwrap();
        assert!(code.contains("package main"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
