//! crabsim's MCP server: every operation of the catalogue as a tool, over stdio, on one models
//! folder. It only translates between MCP and the catalogue, so it has no logic of its own: see
//! `docs/design/operation-catalogue.md`.
//!
//! ```text
//! crabsim-mcp --models C:/Users/me/crabsim-models
//! ```

use clap::Parser;
use crabsim_catalogue::{Catalogue, Effect, Settings, Source};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "crabsim-mcp", about = "crabsim's MCP server over stdio: validate, manage and run models, and read their results")]
struct Opts {
    /// The models folder: the only place crabsim's operations touch. It must not hold source
    /// code, such as a Cargo.toml, a .git or .rs files.
    #[arg(long)]
    models: PathBuf,
}

/// What a harness shows its agent about the server as a whole.
const INSTRUCTIONS: &str = "crabsim is a discrete-event material-flow simulator. Models are JSON files in one models folder, named by their path inside it. \
Start with model_file_format, which explains how models are written and how their objects behave. \
Typical work: list_models, read_model, write_model or validate_model; then start_run, poll run_status until it's done, and read run_results. \
Every run's results stay in a results store, which query_results reads with SQL to compare runs and models.";

struct Server {
    catalogue: Arc<Catalogue>,
    tools: Vec<Tool>,
}

impl Server {
    fn new(catalogue: Catalogue) -> Server {
        let tools = catalogue
            .operations()
            .map(|operation| {
                let annotations = match operation.effect {
                    Effect::ReadOnly => ToolAnnotations::new().read_only(true).open_world(false),
                    Effect::Additive => ToolAnnotations::new().read_only(false).destructive(false).open_world(false),
                    Effect::Destructive => ToolAnnotations::new().read_only(false).destructive(true).open_world(false),
                };
                Tool::new(operation.name, operation.description, Arc::new(object(&operation.input_schema)))
                    .with_raw_output_schema(Arc::new(object(&operation.output_schema)))
                    .with_annotations(annotations)
            })
            .collect();
        Server { catalogue: Arc::new(catalogue), tools }
    }
}

fn object(schema: &Value) -> JsonObject {
    schema.as_object().cloned().expect("every operation's schema is an object")
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("crabsim", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    /// Runs the operation on a thread where blocking is fine. A failed operation is a tool
    /// result marked as an error, holding the error as JSON, so the agent reads it.
    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let catalogue = Arc::clone(&self.catalogue);
        let name = request.name.to_string();
        let input = Value::Object(request.arguments.unwrap_or_default());
        let outcome = tokio::task::spawn_blocking(move || catalogue.call(&name, input))
            .await
            .map_err(|error| ErrorData::internal_error(format!("the operation stopped: {error}"), None))?;
        Ok(match outcome {
            Ok(output) => CallToolResult::structured(output),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_json().to_string())]),
        }
        .into())
    }
}

fn main() -> ExitCode {
    // Logging goes to stderr: stdout carries the protocol.
    let _ = env_logger::Builder::from_default_env().target(env_logger::Target::Stderr).try_init();
    let opts = Opts::parse();
    let catalogue = match Catalogue::open(Settings::new(&opts.models, Source::Mcp)) {
        Ok(catalogue) => catalogue,
        Err(error) => {
            eprintln!("crabsim-mcp can't start: {error}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a tokio runtime starts");
    let served = runtime.block_on(async {
        let service = Server::new(catalogue).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    });
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("crabsim-mcp stopped: {error}");
            ExitCode::FAILURE
        }
    }
}
