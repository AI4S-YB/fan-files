mod commands;
mod skill;

use clap::{Parser, Subcommand};
use fan_core::config::Config;

#[derive(Parser)]
#[command(name = "fan-files", version = "0.1.0", about = "intelligent file metadata search engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon (scan + watch + serve)
    Daemon,
    /// Search files by natural language query
    Search {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// Suggest related datasets for a project directory
    Suggest {
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// List files by category or tag
    List {
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Get detailed metadata for a file
    Info {
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Show index status
    Status,
    /// Run LLM inference on indexed files
    Infer,
    /// List projects, or show details if a project name is given
    Projects {
        /// Optional project name to show details
        name: Option<String>,
    },
    /// Generate Claude Code skill file
    GenerateSkill {
        #[arg(long, default_value = "skill/fan-files.md")]
        output: std::path::PathBuf,
    },
}

fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let config = Config::load().expect("Failed to load config");

    match cli.command {
        Commands::Daemon => commands::daemon::run(&config),
        Commands::Search { query, json } => commands::search::run(&config, &query, json),
        Commands::Suggest { path, json } => commands::suggest::run(&config, &path, json),
        Commands::List { category, tag, json } => commands::list::run(&config, category.as_deref(), tag.as_deref(), json),
        Commands::Info { path, json } => commands::info::run(&config, &path, json),
        Commands::Status => commands::status::run(&config),
        Commands::Infer => commands::infer::run(&config),
        Commands::Projects { name } => commands::projects::run(&config, name.as_deref()),
        Commands::GenerateSkill { output } => skill::run(&output),
    }
}
