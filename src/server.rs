//! The MCP server: Logix Engine's server, with the extension tools added.
//!
//! Engine tools go to the engine's own `EngineServer` unchanged, so they
//! keep its progress, live output and cancellation. Extension tools
//! (src/tools.rs) are listed after the engine's and handled here.

use std::borrow::Cow;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use logix_engine::server::EngineServer;
use logix_engine::state::EngineState;

use crate::tools;

/// Added to the engine's server instructions.
const EXTENSION_INSTRUCTIONS: &str = "This server is soft-agent: Logix Engine with experimental \
    extension tools. Everything above about Logix Engine applies to it unchanged. The \
    extension tools are listed after the engine's; get_soft_agent_info tells which they are.";

pub struct SoftAgentServer {
    engine: EngineServer,
    /// The same state the engine has, for the extension tools.
    state: EngineState,
}

impl SoftAgentServer {
    pub fn new(state: EngineState) -> Self {
        SoftAgentServer {
            engine: EngineServer::new(state.clone()),
            state,
        }
    }
}

impl ServerHandler for SoftAgentServer {
    fn get_info(&self) -> ServerConfig {
        let mut info = self.engine.get_info();
        info.server_info.name = "soft-agent".into();
        info.server_info.title = Some("soft-agent (Logix Engine extensions)".into());
        info.server_info.version = format!(
            "{} (Logix Engine {})",
            env!("CARGO_PKG_VERSION"),
            logix_engine::state::ENGINE_VERSION
        );
        info.server_info.website_url = None;
        info.instructions = Some(match info.instructions.take() {
            Some(engine) => format!("{engine}\n\n{EXTENSION_INSTRUCTIONS}"),
            None => EXTENSION_INSTRUCTIONS.to_string(),
        });
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.engine.supported_protocol_versions()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut all = logix_engine::tools::all_definitions();
        all.extend(tools::all_definitions());
        Ok(ListToolsResult::with_all_items(all))
    }

    fn get_tool(&self, _name: &str) -> Option<Tool> {
        // As the engine does: None skips rmcp's validation, so bad input is
        // a request error the caller can read.
        None
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        // Held until program_board is done: this process's debug sessions on
        // the board stay paused meanwhile.
        let _pause =
            match tools::guard_program_board(&self.state, &name, request.arguments.as_ref()).await
            {
                Ok(pause) => pause,
                Err(refusal) => return Ok(refusal.into()),
            };
        match tools::call(&self.state, &name, request.arguments.clone()).await {
            Some(result) => {
                log::info!("Extension tool call: {name}");
                Ok(result.into())
            }
            None => self.engine.call_tool(request, context).await,
        }
    }
}
