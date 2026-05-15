//! `a2a schema` — inspect A2A protocol types.
//!
//! Schemas are generated at build time from a2a.proto via build.rs and stay
//! in sync with the proto automatically.

use clap::Subcommand;

use crate::cli::GlobalArgs;
use crate::error::Result;
use crate::printer::print_value;

#[derive(Debug, Subcommand)]
pub enum SchemaCommand {
    /// SendMessageRequest — what to pass to `a2a send`
    Send,
    /// Task — the response structure returned by send/get-task
    Task,
    /// AgentCard — the agent's capability declaration
    Card,
}

const SCHEMA_SEND: &str = include_str!(concat!(env!("OUT_DIR"), "/schema_send.json"));
const SCHEMA_TASK: &str = include_str!(concat!(env!("OUT_DIR"), "/schema_task.json"));
const SCHEMA_CARD: &str = include_str!(concat!(env!("OUT_DIR"), "/schema_card.json"));

pub async fn run_schema(cmd: &SchemaCommand, args: &GlobalArgs) -> Result<()> {
    let schema = match cmd {
        SchemaCommand::Send => serde_json::from_str(SCHEMA_SEND)?,
        SchemaCommand::Task => serde_json::from_str(SCHEMA_TASK)?,
        SchemaCommand::Card => serde_json::from_str(SCHEMA_CARD)?,
    };
    print_value(
        &schema,
        args.fields.as_deref(),
        args.format.clone(),
        args.compact,
    )
}
