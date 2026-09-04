use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(name = "subdollar-bench")]
#[command(about = "Autonomous Under-$1 System-Building Benchmark for Budget LLMs", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run an autonomous benchmark on a model using OMP harness
    Run {
        /// Model identifier on OpenRouter (e.g. openrouter/google/gemini-2.5-flash)
        #[arg(short, long)]
        model: String,

        /// Task to run
        #[arg(short, long, value_enum, default_value_t = TaskType::Redis)]
        task: TaskType,

        /// Reasoning effort / thinking level (auto, off, low, medium, high, max)
        #[arg(long, default_value = "auto")]
        effort: String,

        /// Maximum turns/steps allowed for OMP
        #[arg(long, default_value_t = 15)]
        max_turns: u32,

        /// Maximum cost budget in USD (e.g. 0.20 for 20 cents)
        #[arg(long, default_value_t = 0.50)]
        budget_usd: f64,

        /// Maximum execution time in minutes before stopping agent (e.g. 15 for 15m)
        #[arg(long, default_value_t = 15)]
        timeout_min: u64,

        /// OpenRouter API Key (optional, defaults to OPENROUTER_API_KEY env var)
        #[arg(long, env = "OPENROUTER_API_KEY")]
        api_key: Option<String>,

        /// Path to workspace directory for the generated project
        #[arg(long, default_value = "./workspace")]
        workdir: String,

        /// Skip OMP agent run and evaluate existing server directly
        #[arg(long, default_value_t = false)]
        eval_only: bool,

        /// Number of repeated trials for statistical variance (Pass@k)
        #[arg(long, default_value_t = 1)]
        trials: u32,
    },

    /// Directly run verification test suite against a running server
    Eval {
        /// Task to verify
        #[arg(short, long, value_enum)]
        task: TaskType,

        /// Target port
        #[arg(short, long)]
        port: Option<u16>,
    },

    /// Display current benchmark leaderboard from saved results
    Leaderboard {
        /// Directory containing result JSON files
        #[arg(short, long, default_value = "./results")]
        results_dir: String,
    },

    /// Publish a completed run to Git repository and update SUMMARY.md
    Publish {
        /// ID of the run to publish (e.g. redis_openrouter_meta_muse-spark-1.3-contributor_20260904_085848)
        #[arg(short, long)]
        run_id: String,

        /// Optional custom git commit message
        #[arg(short, long)]
        message: Option<String>,

        /// Path to runs directory
        #[arg(long, default_value = "./runs")]
        runs_dir: String,

        /// Path to results directory
        #[arg(long, default_value = "./results")]
        results_dir: String,

        /// Path to repository root
        #[arg(long, default_value = ".")]
        repo_root: String,
    },

    /// Regenerate SUMMARY.md from recorded runs
    Summary {
        /// Path to runs directory
        #[arg(long, default_value = "./runs")]
        runs_dir: String,

        /// Path to repository root
        #[arg(long, default_value = ".")]
        repo_root: String,
    },

    /// Launch web-based GUI for configuring and running benchmarks
    Ui {
        /// Port to bind the web server
        #[arg(short, long, default_value_t = 3000)]
        port: u16,

        /// Host address to bind
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskType {
    Redis,
    Http,
    Dns,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Redis => write!(f, "redis"),
            TaskType::Http => write!(f, "http"),
            TaskType::Dns => write!(f, "dns"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_type_display_and_serde() {
        assert_eq!(format!("{}", TaskType::Redis), "redis");
        assert_eq!(format!("{}", TaskType::Http), "http");
        assert_eq!(format!("{}", TaskType::Dns), "dns");

        let json_redis = serde_json::to_string(&TaskType::Redis).unwrap();
        assert_eq!(json_redis, "\"redis\"");
        let de_redis: TaskType = serde_json::from_str(&json_redis).unwrap();
        assert_eq!(de_redis, TaskType::Redis);

        let json_dns = serde_json::to_string(&TaskType::Dns).unwrap();
        assert_eq!(json_dns, "\"dns\"");
        let de_dns: TaskType = serde_json::from_str(&json_dns).unwrap();
        assert_eq!(de_dns, TaskType::Dns);

        let json_http = serde_json::to_string(&TaskType::Http).unwrap();
        assert_eq!(json_http, "\"http\"");
        let de_http: TaskType = serde_json::from_str(&json_http).unwrap();
        assert_eq!(de_http, TaskType::Http);
    }

    #[test]
    fn test_cli_parsing_run() {
        let args = ["subdollar-bench", "run", "--model", "test-model", "--task", "redis"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Run { model, task, effort, budget_usd, max_turns, timeout_min, eval_only, .. } => {
                assert_eq!(model, "test-model");
                assert_eq!(task, TaskType::Redis);
                assert_eq!(effort, "auto");
                assert_eq!(budget_usd, 0.50);
                assert_eq!(max_turns, 15);
                assert_eq!(timeout_min, 15);
                assert!(!eval_only);
            }
            _ => panic!("Expected Run command"),
        }

        let args_http = ["subdollar-bench", "run", "--model", "test-model-2", "--task", "http", "--effort", "max", "--budget-usd", "0.20", "--max-turns", "10", "--timeout-min", "5", "--eval-only"];
        let cli_http = Cli::try_parse_from(args_http).unwrap();
        match cli_http.command {
            Commands::Run { model, task, effort, budget_usd, max_turns, timeout_min, eval_only, .. } => {
                assert_eq!(model, "test-model-2");
                assert_eq!(task, TaskType::Http);
                assert_eq!(effort, "max");
                assert_eq!(budget_usd, 0.20);
                assert_eq!(max_turns, 10);
                assert_eq!(timeout_min, 5);
                assert!(eval_only);
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_cli_parsing_eval() {
        let args = ["subdollar-bench", "eval", "--task", "redis", "--port", "6379"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Eval { task, port } => {
                assert_eq!(task, TaskType::Redis);
                assert_eq!(port, Some(6379));
            }
            _ => panic!("Expected Eval command"),
        }

        let args2 = ["subdollar-bench", "eval", "--task", "http"];
        let cli2 = Cli::try_parse_from(args2).unwrap();
        match cli2.command {
            Commands::Eval { task, port } => {
                assert_eq!(task, TaskType::Http);
                assert_eq!(port, None);
            }
            _ => panic!("Expected Eval command"),
        }
    }

    #[test]
    fn test_cli_parsing_leaderboard() {
        let args = ["subdollar-bench", "leaderboard", "--results-dir", "./custom_results"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Leaderboard { results_dir } => {
                assert_eq!(results_dir, "./custom_results");
            }
            _ => panic!("Expected Leaderboard command"),
        }
    }

    #[test]
    fn test_cli_parsing_publish() {
        let args = ["subdollar-bench", "publish", "--run-id", "test_run_123", "--message", "custom commit msg"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Publish { run_id, message, runs_dir, results_dir, repo_root } => {
                assert_eq!(run_id, "test_run_123");
                assert_eq!(message, Some("custom commit msg".to_string()));
                assert_eq!(runs_dir, "./runs");
                assert_eq!(results_dir, "./results");
                assert_eq!(repo_root, ".");
            }
            _ => panic!("Expected Publish command"),
        }
    }

    #[test]
    fn test_cli_parsing_summary() {
        let args = ["subdollar-bench", "summary", "--runs-dir", "./test_runs", "--repo-root", "./test_repo"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Summary { runs_dir, repo_root } => {
                assert_eq!(runs_dir, "./test_runs");
                assert_eq!(repo_root, "./test_repo");
            }
            _ => panic!("Expected Summary command"),
        }
    }

    #[test]
    fn test_cli_parsing_ui() {
        let args = ["subdollar-bench", "ui", "--port", "8080", "--host", "127.0.0.1"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Ui { port, host } => {
                assert_eq!(port, 8080);
                assert_eq!(host, "127.0.0.1");
            }
            _ => panic!("Expected Ui command"),
        }
    }
}
