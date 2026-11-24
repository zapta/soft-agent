//! soft-agent: Logix Engine's MCP server with extension tools, over
//! stdin/stdout. Same command line as logix-engine (its
//! docs/command-line.md).
//!
//! The startup below follows logix-engine's src/main.rs, which is in the
//! engine's binary, not its library. Keep it in step when bumping the
//! engine (README.md, Engine version).

mod server;
mod tools;

use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;

use logix_engine::packages::{PackageManager, manifest, remote_config};
use logix_engine::state::{self, EngineState};
use logix_engine::{catalog, logging, paths};

/// soft-agent: Logix Engine plus experimental tools. A host launches it as
/// a child process and talks MCP with it over stdin and stdout.
#[derive(Parser, Debug)]
#[command(name = "soft-agent", version = env!("CARGO_PKG_VERSION"))]
struct Args {
    /// The host's engine folder, as an absolute path. Its last folder name
    /// must contain "engine", such as ~/.myhost/engine.
    #[arg(long, value_name = "absolute path")]
    engine_folder: String,

    /// Turn off automatic package download and repair.
    #[arg(long)]
    no_auto_download: bool,

    /// How much goes into the log file.
    #[arg(long, value_name = "level", default_value = "info")]
    log_level: LogLevel,

    /// The host's log session folder, as an absolute path. Without it, the
    /// server makes its own session folder in <engine folder>/logs.
    #[arg(long, value_name = "absolute path")]
    log_folder: Option<String>,

    /// The log name in the host's session folder. Needs --log-folder;
    /// defaults to engine.
    #[arg(long, value_name = "name")]
    log_name: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl From<LogLevel> for log::LevelFilter {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
        }
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("soft-agent: {message}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    // First, before any thread starts.
    logging::init_local_offset();
    let command_line: Vec<String> = std::env::args().skip(1).collect();
    // The engine runs its own binary (std::env::current_exe, which is this
    // one) as the diagram layout worker and, on Windows, as the driver
    // helper. Both must be handled here, as the engine's main does.
    if let [option, bundle] = command_line.as_slice()
        && option == logix_engine::diagram::layout::WORKER_OPTION
    {
        return logix_engine::diagram::layout::worker_main(Path::new(bundle));
    }
    if let [option, request] = command_line.as_slice()
        && option == logix_engine::elevated::OPTION
    {
        return logix_engine::elevated::helper_main(Path::new(request));
    }
    let args = Args::parse();

    let resolved =
        match paths::check_engine_folder(&args.engine_folder, paths::home_folder().as_deref()) {
            Ok(p) => p,
            Err(e) => return fail(&e),
        };
    if let Err(e) = std::fs::create_dir_all(&resolved) {
        return fail(&format!(
            "can't create the engine folder {:?}: {e}",
            args.engine_folder
        ));
    }
    let log_folder = match args.log_folder.as_deref().map(paths::check_absolute_path) {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => return fail(&format!("--log-folder: {e}")),
        None => None,
    };
    let (place, at_top) = match logging::place_for(
        Path::new(&args.engine_folder),
        log_folder.as_deref(),
        args.log_name.as_deref(),
    ) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let log_file = place.file();
    let resolved_place = if at_top {
        logging::LogPlace {
            folder: resolved
                .join("logs")
                .join(place.folder.file_name().unwrap_or_default()),
            name: place.name.clone(),
        }
    } else {
        place
    };
    if let Err(e) = logging::init(resolved_place, at_top, args.log_level.into()) {
        return fail(&format!("can't create the log file: {e}"));
    }

    // Packages: the engine's remote config, or its diagnostic overrides.
    if let Err(e) = remote_config::channel_variable() {
        return fail(&e);
    }
    if let Ok(Some(channel)) = remote_config::channel_variable() {
        log::warn!(
            "Using the channel {channel} ({})",
            remote_config::CHANNEL_VAR
        );
    }
    let source = match remote_config::ConfigSource::from_env() {
        Some(o) => {
            let is_url = o.location.starts_with("https://") || o.location.starts_with("http://");
            if !is_url && !Path::new(&o.location).is_dir() {
                return fail(&format!(
                    "{}: no such folder: {}",
                    remote_config::DEV_REMOTE_CONFIG_VAR,
                    o.location
                ));
            }
            log::warn!(
                "Using the remote config {} ({})",
                o.location,
                remote_config::DEV_REMOTE_CONFIG_VAR
            );
            o
        }
        None => remote_config::ConfigSource::regular(),
    };
    let packages = PackageManager::with_remote_config(
        &resolved,
        source,
        !args.no_auto_download,
        &state::platform(),
        manifest::GITHUB,
    );
    if let Some(folder) = std::env::var_os(catalog::DEV_CATALOG_VAR).filter(|v| !v.is_empty()) {
        if let Err(e) = catalog::load(Path::new(&folder), "local", true) {
            return fail(&format!("{}: {e}", catalog::DEV_CATALOG_VAR));
        }
        log::warn!(
            "Using the board catalog in {} ({})",
            Path::new(&folder).display(),
            catalog::DEV_CATALOG_VAR
        );
    }
    let state = EngineState {
        engine_folder: args.engine_folder.clone(),
        engine_folder_resolved: resolved,
        packages,
        auto_download: !args.no_auto_download,
        command_line,
        log_file,
    };
    log::info!(
        "soft-agent {} on Logix Engine {} ({}) started, pid {}, arguments {:?}",
        env!("CARGO_PKG_VERSION"),
        state::ENGINE_VERSION,
        state::platform(),
        std::process::id(),
        state.command_line
    );

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => return fail(&format!("can't start: {e}")),
    };
    let code = runtime.block_on(async {
        let service = match server::SoftAgentServer::new(state)
            .serve(rmcp::transport::stdio())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                log::error!("MCP session failed to start: {e}");
                return ExitCode::FAILURE;
            }
        };
        match service.waiting().await {
            Ok(reason) => {
                log::info!("MCP session ended: {reason:?}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                log::error!("MCP session failed: {e}");
                ExitCode::FAILURE
            }
        }
    });
    log::logger().flush();
    code
}
