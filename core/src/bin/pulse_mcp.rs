//! `pulse-mcp` — MCP server exposing pulse-daemon over stdio. Each tool
//! maps directly to a protocol::Request variant and forwards through
//! the same client::send used by the CLI (see docs/protocol.md).

use std::collections::HashMap;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;

use pulse_core::client;
use pulse_core::protocol::Request;

#[derive(Debug, Deserialize, JsonSchema)]
struct ListNotesParams {
    #[schemars(description = "Only return notes whose properties match these key/value pairs")]
    filter: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AddNoteParams {
    #[schemars(description = "The note's text content")]
    text: String,
    #[schemars(description = "Arbitrary key/value tags for the note")]
    #[serde(default)]
    properties: HashMap<String, String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateNoteParams {
    #[schemars(description = "The id of the note to update")]
    id: String,
    #[schemars(description = "New text; omit to leave the text unchanged")]
    text: Option<String>,
    #[schemars(description = "Properties to replace entirely; omit to leave them unchanged")]
    properties: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteNoteParams {
    #[schemars(description = "The id of the note to delete")]
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetIntervalParams {
    #[schemars(description = "How often notes should surface, in seconds")]
    seconds: u64,
}

#[derive(Clone)]
struct PulseMcpServer {
    // Read internally by the #[tool_handler]-generated dispatch, not by
    // this file directly — the compiler's dead-code pass doesn't see
    // through the macro expansion.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl PulseMcpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List notes, optionally filtered by property key/value pairs")]
    async fn list_notes(
        &self,
        Parameters(ListNotesParams { filter }): Parameters<ListNotesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        send_request(Request::ListNotes { filter }).await
    }

    #[tool(description = "Add a new note")]
    async fn add_note(
        &self,
        Parameters(AddNoteParams { text, properties }): Parameters<AddNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        send_request(Request::AddNote { text, properties }).await
    }

    #[tool(description = "Update an existing note's text and/or properties")]
    async fn update_note(
        &self,
        Parameters(UpdateNoteParams {
            id,
            text,
            properties,
        }): Parameters<UpdateNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_note_id(&id)?;
        send_request(Request::UpdateNote {
            id,
            text,
            properties,
        })
        .await
    }

    #[tool(description = "Delete a note by id")]
    async fn delete_note(
        &self,
        Parameters(DeleteNoteParams { id }): Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = parse_note_id(&id)?;
        send_request(Request::DeleteNote { id }).await
    }

    #[tool(description = "Get how often notes currently surface, in seconds")]
    async fn get_interval(&self) -> Result<CallToolResult, ErrorData> {
        send_request(Request::GetInterval).await
    }

    #[tool(description = "Set how often notes should surface, in seconds")]
    async fn set_interval(
        &self,
        Parameters(SetIntervalParams { seconds }): Parameters<SetIntervalParams>,
    ) -> Result<CallToolResult, ErrorData> {
        send_request(Request::SetInterval { seconds }).await
    }
}

#[tool_handler]
impl ServerHandler for PulseMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Manage pulse notes: list, add, update, delete, and adjust the surfacing interval.",
        )
    }
}

async fn send_request(request: Request) -> Result<CallToolResult, ErrorData> {
    let response = client::send(&request)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    let text = serde_json::to_string_pretty(&response)
        .unwrap_or_else(|_| response.to_string());
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn parse_note_id(id: &str) -> Result<uuid::Uuid, ErrorData> {
    id.parse()
        .map_err(|_| ErrorData::invalid_params(format!("'{id}' is not a valid note id"), None))
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let service = match PulseMcpServer::new().serve(rmcp::transport::stdio()).await {
        Ok(service) => service,
        Err(e) => {
            eprintln!("pulse-mcp: failed to start: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(e) = service.waiting().await {
        eprintln!("pulse-mcp: {e}");
        return std::process::ExitCode::FAILURE;
    }

    std::process::ExitCode::SUCCESS
}
