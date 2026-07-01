mod commands;
mod version;
mod version_check;

use clap::{Parser, Subcommand};
use fan_core::config::Config;

#[derive(Parser)]
#[command(name = "fan-files", version = "0.1.0", about = "intelligent file metadata search engine")]
struct Cli {
    /// Use global (admin-managed) data layer
    #[arg(long, global = true)]
    global: bool,

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
        server: Option<String>,
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
        #[command(subcommand)]
        action: Option<ProjectAction>,
    },
    /// Show or clear pending review items
    Pending {
        #[arg(long)]
        clear: bool,
    },
    /// Update fan-files to the latest version
    Update,
    /// Uninstall fan-files
    Uninstall,
    /// Interactive setup wizard
    Init,
    /// Manage registered servers
    #[command(subcommand)]
    Servers(ServersAction),
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Show project details
    Show {
        name: String,
    },
    /// Update project metadata
    Update {
        name: String,
        #[arg(long)]
        species: Option<String>,
        #[arg(long, value_name = "high|medium|low")]
        confidence: Option<String>,
        #[arg(long)]
        assay_type: Option<String>,
    },
}

#[derive(Subcommand)]
enum ServersAction {
    /// List all registered servers
    List,
    /// Add a new server (interactive)
    Add {
        name: String,
    },
    /// Remove a server
    Remove {
        name: String,
    },
    /// Scan a single server (use --agent for fan-agent mode)
    Scan {
        name: String,
        #[arg(long)]
        agent: bool,
    },
    /// Real-time watch a remote server (requires fan-agent)
    Watch {
        name: String,
    },
}

fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    // Async version check (non-blocking)
    version_check::spawn_check();

    let layer = resolve_layer(cli.global);
    let config_path = config_path_for(&layer);
    let config = Config::load_from(&config_path).unwrap_or_else(|_| {
        eprintln!("Warning: no config at {}, using defaults", config_path.display());
        Config::default()
    });

    match cli.command {
        Commands::Daemon => commands::daemon::run(&config, &layer),
        Commands::Search { query, json } => commands::search::run(&config, &layer, &query, json),
        Commands::Suggest { path, json } => commands::suggest::run(&config, &path, json),
        Commands::List { category, tag, server, json } => commands::list::run(&config, &layer, category.as_deref(), tag.as_deref(), server.as_deref(), json),
        Commands::Info { path, json } => commands::info::run(&config, &layer, &path, json),
        Commands::Status => commands::status::run(&config, &layer),
        Commands::Infer => commands::infer::run(&config, &layer),
        Commands::Projects { action } => match action {
            Some(ProjectAction::Show { name }) => commands::projects::run(&config, &layer, Some(name.as_str())),
            Some(ProjectAction::Update { name, species, confidence, assay_type }) => {
                commands::projects::run_update(&config, &name, species.as_deref(), confidence.as_deref(), assay_type.as_deref())
            }
            None => commands::projects::run(&config, &layer, None),
        },
        Commands::Pending { clear } => commands::pending::run(clear),
        Commands::Update => commands::update::run(),
        Commands::Uninstall => commands::uninstall::run(),
        Commands::Init => commands::init::run(&config, &layer),
        Commands::Servers(action) => match action {
            ServersAction::List => commands::servers::list(&config),
            ServersAction::Add { name } => commands::servers::add(&name),
            ServersAction::Remove { name } => commands::servers::remove(&name),
            ServersAction::Scan { name, agent } => commands::servers::scan_one_inner(&name, agent),
            ServersAction::Watch { name } => commands::servers::watch_remote(&name),
        },
    }
}

fn resolve_layer(global: bool) -> fan_core::config::DataLayer {
    if global {
        fan_core::config::DataLayer::Global
    } else {
        fan_core::config::DataLayer::User
    }
}

fn config_path_for(layer: &fan_core::config::DataLayer) -> std::path::PathBuf {
    match layer {
        fan_core::config::DataLayer::Global => fan_core::config::config_path_global(),
        fan_core::config::DataLayer::User => fan_core::config::dirs_fan().join("config.toml"),
    }
}
