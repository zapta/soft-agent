//! The extension tools. Each one is a `logix_engine::tools::EngineTool`, so
//! its schema, input checks and results work the same as the engine's.
//!
//! To add a tool: define its type and input/output structs like
//! `GetSoftAgentInfo`, add it to `all_definitions` and to `call`.

use rmcp::model::{CallToolResult, JsonObject, Tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logix_engine::state::{self, EngineState};
use logix_engine::tools::{EngineTool, definition, parse_input, to_call_result};

/// The extension tools, in the order `tools/list` returns them (after the
/// engine's).
pub fn all_definitions() -> Vec<Tool> {
    vec![definition::<GetSoftAgentInfo>()]
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
