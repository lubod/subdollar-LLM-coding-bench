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
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Redis => write!(f, "redis"),
            TaskType::Http => write!(f, "http"),
        }
    }
}
