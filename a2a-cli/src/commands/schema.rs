//! `a2a schema` — inspect A2A protocol types.
//!
//! Schemas are generated at build time from a2a.proto via build.rs and stay
//! in sync with the proto automatically.

use clap::Subcommand;

use crate::cli::GlobalArgs;
use crate::error::Result;
use crate::printer::print_json;

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

pub async fn run_schema(cmd: &SchemaCommand, _args: &GlobalArgs) -> Result<()> {
    match cmd {
        SchemaCommand::Send => print_json(&serde_json::from_str(SCHEMA_SEND)?, None, false),
        SchemaCommand::Task => print_json(&serde_json::from_str(SCHEMA_TASK)?, None, false),
        SchemaCommand::Card => print_json(&serde_json::from_str(SCHEMA_CARD)?, None, false),
    }
}
