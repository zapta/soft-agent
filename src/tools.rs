//! The extension tools. Each one is a `logix_engine::tools::EngineTool`, so
//! its schema, input checks and results work the same as the engine's.
//!
//! To add a tool: define its type and input/output structs like
//! `GetSoftAgentInfo`, add it to `all_definitions` and to `call`.

use rmcp::model::{CallToolResult, JsonObject, Tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logix_engine::progress::NoProgress;
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, EngineTool, definition, parse_input, to_call_result};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::debug::build::DebugBuild;
use crate::debug::panel::DebugPanel;
use crate::debug::session::{self, DebugClose, DebugOpen, DebugRead, DebugWrite};

/// The extension tools, in the order `tools/list` returns them (after the
/// engine's).
pub fn all_definitions() -> Vec<Tool> {
    vec![
        definition::<GetSoftAgentInfo>(),
        definition::<DebugBuild>(),
        definition::<DebugOpen>(),
        definition::<DebugRead>(),
        definition::<DebugWrite>(),
        definition::<DebugClose>(),
        definition::<DebugPanel>(),
    ]
}

/// Before program_board: refuses a board that is in a debug session in
/// another soft-agent process (its lock file, crate::debug::lock), since
/// programming it would change the design under that session. Otherwise
/// pauses this process's sessions on the board until the returned guard
/// is dropped, after programming; they reconnect and check the build
/// again. Ok(None) for other tools, or when the board isn't found (the
/// engine then reports it).
pub async fn guard_program_board(
    state: &EngineState,
    name: &str,
    arguments: Option<&JsonObject>,
) -> Result<Option<session::Pause>, CallToolResult> {
    use crate::debug::lock;
    use logix_engine::tools::list_devices;
    if name != "program_board" {
        return Ok(None);
    }
    let Some(device) = arguments
        .and_then(|a| a.get("device"))
        .and_then(|d| d.as_str())
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let state = state.clone();
    let found = tokio::task::spawn_blocking(move || {
        let devices = list_devices::run(&state, Default::default()).ok()?.devices;
        let d = devices
            .iter()
            .find(|d| d.id == device || list_devices::same_device(&d.id, &device))?;
        let ports: Vec<String> = d
            .serial_ports
            .iter()
            .map(|p| p.port.clone())
            .chain(d.serial_port.clone())
            .collect();
        let own = session::session_ports();
        let refusal = ports
            .iter()
            .filter(|p| !own.iter().any(|o| lock::same_port(o, p)))
            .find_map(|p| lock::holder_of(p).map(|h| (p.clone(), h)))
            .map(|(port, h)| {
                logix_engine::errors::ToolError::setup(format!(
                    "The board on {port} is in a debug session in another program{}: \
                     programming it would change the design under that session.",
                    h.process()
                ))
                .with_details(
                    "Close that session first: press Enter in the terminal of debug_run.py \
                     or debug_panel.py, or call close_debug in the program that opened it.",
                )
            });
        Some(match refusal {
            Some(r) => Err(r),
            None => Ok(session::pause(&ports)),
        })
    })
    .await
    .ok()
    .flatten();
    match found {
        None => Ok(None),
        Some(Ok(pause)) => Ok(Some(pause)),
        Some(Err(refusal)) => {
            log::info!("program_board refused: {}", refusal.message);
            let display = Default::default();
            Err(to_call_result::<()>(&display, Err(refusal)))
        }
    }
}

/// Runs an extension tool by name. Returns None when the name isn't one,
/// so the engine handles it.
pub async fn call(
    state: &EngineState,
    name: &str,
    arguments: Option<JsonObject>,
) -> Option<CallToolResult> {
    let display = Default::default();
    Some(match name {
        GetSoftAgentInfo::NAME => to_call_result(
            &display,
            parse_input::<GetSoftAgentInfo>(arguments).map(|_| get_soft_agent_info(state)),
        ),
        DebugBuild::NAME => {
            let call = Call::new(Arc::new(NoProgress), CancellationToken::new());
            let result = match parse_input::<DebugBuild>(arguments) {
                Ok(input) => crate::debug::build::run(state, input, &call).await,
                Err(e) => Err(e),
            };
            to_call_result(&display, result)
        }
        DebugOpen::NAME => to_call_result(
            &display,
            parse_input::<DebugOpen>(arguments).and_then(session::open),
        ),
        DebugRead::NAME => to_call_result(
            &display,
            parse_input::<DebugRead>(arguments).and_then(session::read),
        ),
        DebugWrite::NAME => to_call_result(
            &display,
            parse_input::<DebugWrite>(arguments).and_then(session::write),
        ),
        DebugPanel::NAME => {
            let result = match parse_input::<DebugPanel>(arguments) {
                Ok(input) => crate::debug::panel::run(input).await,
                Err(e) => Err(e),
            };
            to_call_result(&display, result)
        }
        DebugClose::NAME => to_call_result(
            &display,
            parse_input::<DebugClose>(arguments).and_then(session::close),
        ),
        _ => return None,
    })
}

// --- get_soft_agent_info ---------------------------------------------------

pub struct GetSoftAgentInfo;

impl EngineTool for GetSoftAgentInfo {
    const NAME: &'static str = "get_soft_agent_info";
    const DESCRIPTION: &'static str = "Returns the soft-agent extension's version, the Logix \
        Engine version it is built on, and the names of the extension tools (the other tools \
        are the engine's). Fast and local.";
    const READ_ONLY: bool = true;
    type Input = GetSoftAgentInfoInput;
    type Output = GetSoftAgentInfoOutput;
}

/// The get_soft_agent_info tool takes no inputs.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSoftAgentInfoInput {}

/// The extension's version and tools.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetSoftAgentInfoOutput {
    /// The soft-agent version.
    pub extension_version: String,
    /// The Logix Engine version it is built on, as yyyy.mm.dd.
    pub engine_version: String,
    /// The engine commit it is built on, short; omitted when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_commit: Option<String>,
    /// The engine folder, as given with --engine-folder.
    pub engine_folder: String,
    /// The extension tools' names.
    pub extension_tools: Vec<String>,
}

fn get_soft_agent_info(state: &EngineState) -> GetSoftAgentInfoOutput {
    GetSoftAgentInfoOutput {
        extension_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: state::ENGINE_VERSION.to_string(),
        engine_commit: Some(state::ENGINE_COMMIT.to_string()).filter(|c| !c.is_empty()),
        engine_folder: state.engine_folder.clone(),
        extension_tools: all_definitions()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect(),
    }
}
