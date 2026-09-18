use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum JevCommand {
    /// Save, inspect or remove your TypeSafe API credential
    Auth {
        #[command(subcommand)]
        command: crate::auth::AuthCommand,
    },
    /// Evaluate code quality with TypeSafe (uploads selected source); advisory findings do not fail CI
    Check(Box<CheckArgs>),
    /// Print the versioned rule catalog as JSON
    Rules,
    /// Serve read-only snapshots on localhost (run alongside check --watch)
    Serve {
        #[arg(long, default_value_t = 47831)]
        port: u16,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Format {
    Agent,
    Json,
    Jsonl,
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Files/directories relative to the invocation; default: discovered source and tests
    pub paths: Vec<PathBuf>,
    /// Review working-tree changes against this Git revision (includes staged and untracked files)
    #[arg(long)]
    pub base: Option<String>,
    /// Compatibility flag; default maintainability already uses one batch per file
    #[arg(long)]
    pub quick: bool,
    /// Additional file extension to review as text (repeatable, without a dot)
    #[arg(long, value_parser = source_extension)]
    pub source_extension: Vec<String>,
    /// Related file or contract to include in every request (repeatable, inside root)
    #[arg(long)]
    pub context: Vec<PathBuf>,
    /// Total context bytes per request, of explicitly supplied files; never truncated
    #[arg(long, default_value_t = 32768, value_parser = clap::value_parser!(u64).range(1..=1048576))]
    pub max_context_bytes: u64,
    /// Keep watching saves; write latest.json and emit successive snapshots
    #[arg(long)]
    pub watch: bool,
    /// Save a local HTML dashboard and open it in your browser (updates while watching)
    #[arg(long, conflicts_with = "dry_run")]
    pub report: bool,
    /// List scope without credentials, network requests, or writing state
    #[arg(long)]
    pub dry_run: bool,
    /// Include initial request bodies (selected source and questions) in a dry run
    #[arg(long, requires = "dry_run")]
    pub show_requests: bool,
    /// Output format (watch defaults to jsonl; one-shot defaults to agent)
    #[arg(long, value_enum)]
    pub format: Option<Format>,
    /// TypeSafe model (pin a version for repeatable policy)
    #[arg(long, default_value = "jev-1.13.0")]
    pub model: String,
    /// Credential file (default: repository root/.env); environment key takes precedence
    #[arg(long)]
    pub env_file: Option<PathBuf>,
    /// Optional API attempt ceiling for this invocation, including watch updates
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000000))]
    pub max_requests: Option<u32>,
    /// Maximum simultaneous independent TypeSafe requests (questions within each call are parallel)
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(1..=16))]
    pub concurrency: u32,
    /// Per-file source limit; oversized files are incomplete, never truncated
    #[arg(long, default_value_t = 65536, value_parser = clap::value_parser!(u64).range(1..=1048576))]
    pub max_file_bytes: u64,
    /// Cache lifetime; unchanged watch snapshots are not automatically reevaluated
    #[arg(long, default_value_t = 3600)]
    pub cache_ttl_secs: u64,
    /// Ignore disk cache for this invocation (unchanged watch files still reuse results)
    #[arg(long)]
    pub refresh: bool,
    /// Reuse valid cached responses only; never contact TypeSafe
    #[arg(long, conflicts_with = "refresh")]
    pub cache_only: bool,
    /// Wait this long after changes settle before evaluating
    #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(50..=60000))]
    pub debounce_ms: u64,
    /// Poll interval for watch mode
    #[arg(long, default_value_t = 250, value_parser = clap::value_parser!(u64).range(50..=60000))]
    pub poll_ms: u64,
    /// Enable a rule ID or catalog key (repeatable); defaults to file organization, function simplification and shared logic
    #[arg(long = "rule")]
    pub rules: Vec<String>,
}

fn source_extension(value: &str) -> Result<String, String> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Use an extension without a dot, for example: --source-extension zig".into());
    }
    Ok(value.to_ascii_lowercase())
}

impl CheckArgs {
    pub fn output_format(&self) -> Format {
        self.format.unwrap_or(if self.show_requests {
            Format::Json
        } else if self.watch {
            Format::Jsonl
        } else {
            Format::Agent
        })
    }
}
