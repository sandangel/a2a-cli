//! `agc schema` — inspect A2A protocol types and live agent skill schemas.
//!
//! Usage:
//!   agc schema                       # overview of A2A types
//!   agc schema send                  # SendMessageRequest structure
//!   agc schema task                  # Task response structure
//!   agc schema card                  # AgentCard structure
//!   agc schema skill <id>            # input/output schema for a specific skill

use clap::{Args, Subcommand};
use serde_json::json;

use crate::cli::GlobalArgs;
use crate::error::{AgcError, Result};
use crate::printer::print_json;
use crate::runner::fetch_card;

#[derive(Debug, Subcommand)]
pub enum SchemaCommand {
    /// SendMessageRequest — what to pass to `agc send`
    Send,
    /// Task — the response structure returned by send/get-task
    Task,
    /// AgentCard — the agent's capability declaration
    Card,
    /// Skill input/output schema fetched from the live agent card
    Skill(SkillArgs),
}

#[derive(Debug, Args)]
pub struct SkillArgs {
    /// Skill ID to inspect (from `agc card` or `agc generate-skills`)
    pub id: String,
}

pub async fn run_schema(cmd: &SchemaCommand, args: &GlobalArgs) -> Result<()> {
    match cmd {
        SchemaCommand::Send => print_json(&send_schema(), None, false),
        SchemaCommand::Task => print_json(&task_schema(), None, false),
        SchemaCommand::Card => print_json(&card_schema(), None, false),
        SchemaCommand::Skill(a) => skill_schema(&a.id, args).await,
    }
}

// ── Live skill schema ─────────────────────────────────────────────────

async fn skill_schema(skill_id: &str, args: &GlobalArgs) -> Result<()> {
    let target = crate::client::resolve_target(args)?;
    let card = fetch_card(&target.url, args.bearer_token.as_deref()).await?;

    let skill = card.skills.iter().find(|s| s.id == skill_id).ok_or_else(|| {
        AgcError::InvalidInput(format!(
            "skill {skill_id:?} not found on agent {:?}.\nAvailable: {}",
            target.alias,
            card.skills.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")
        ))
    })?;

    let out = json!({
        "skill_id": skill.id,
        "name": skill.name,
        "description": skill.description,
        "input_modes": skill.input_modes,
        "output_modes": skill.output_modes,
        "tags": skill.tags,
        "examples": skill.examples,
        "how_to_use": {
            "note": "Skills cannot be invoked directly by ID. Send a natural-language message describing what you want — the agent routes internally based on message content.",
            "guidance": "Use the skill's description, examples, and input_modes to craft your message.",
            "command_template": format!("agc --agent {} send \"<message describing what you want>\"", target.alias),
            "example_commands": skill.examples.as_ref().map(|ex| {
                ex.iter().map(|e| format!("agc --agent {} send {:?}", target.alias, e)).collect::<Vec<_>>()
            }).unwrap_or_default()
        }
    });

    print_json(&out, args.fields.as_deref(), args.compact)
}

// ── Static A2A type schemas ───────────────────────────────────────────

fn send_schema() -> serde_json::Value {
    json!({
        "description": "Structure passed to `agc send`. Most fields are optional — only `text` is required.",
        "usage": "agc [--agent <alias>] send \"<text>\" [flags]",
        "fields": {
            "text": {
                "flag": "<positional argument>",
                "type": "string",
                "required": true,
                "description": "The message text to send to the agent."
            },
            "context_id": {
                "flag": "--context-id",
                "type": "string",
                "required": false,
                "description": "Continue an existing conversation context. Omit to start a new one."
            },
            "task_id": {
                "flag": "--task-id",
                "type": "string",
                "required": false,
                "description": "Resume a specific existing task."
            },
            "history_length": {
                "flag": "--history-length",
                "type": "integer",
                "required": false,
                "description": "Ask the agent to include up to N history items in the response."
            },
            "return_immediately": {
                "flag": "--return-immediately",
                "type": "bool",
                "required": false,
                "description": "Return immediately without waiting for completion. Poll status with `agc get-task <id>`."
            },
            "accepted_output_modes": {
                "flag": "--accept-output <mime>",
                "type": "array of strings",
                "required": false,
                "description": "MIME types you accept, e.g. text/plain or application/json. Repeatable."
            }
        },
        "examples": [
            "agc send \"Summarise this PR\"",
            "agc send \"Continue the analysis\" --context-id abc123",
            "agc send \"Generate a report\" --return-immediately",
            "agc --agent prod send \"Status?\" --fields status.message.parts"
        ]
    })
}

fn task_schema() -> serde_json::Value {
    json!({
        "description": "A Task is the result of `agc send` or retrievable via `agc get-task <id>`.",
        "fields": {
            "id": {
                "type": "string",
                "description": "Unique task identifier. Use with `agc get-task`, `agc cancel-task`, `agc subscribe`."
            },
            "context_id": {
                "type": "string",
                "description": "Conversation context this task belongs to."
            },
            "status": {
                "type": "object",
                "fields": {
                    "state": {
                        "type": "enum",
                        "values": ["submitted", "working", "completed", "failed", "canceled", "input-required", "rejected", "auth-required"],
                        "description": "Current task state. Poll until 'completed' or 'failed'."
                    },
                    "message": {
                        "type": "object",
                        "description": "Agent's latest message (present when state = completed or input-required)."
                    }
                }
            },
            "artifacts": {
                "type": "array",
                "description": "Output artifacts produced by the task (files, data, etc.)."
            },
            "history": {
                "type": "array",
                "description": "Message history, included when --history-length > 0."
            }
        },
        "useful_field_paths": {
            "just_the_answer": "--fields status.message.parts",
            "task_state": "--fields id,status.state",
            "check_complete": "--fields id,status.state,status.message"
        },
        "examples": [
            "agc get-task <id>",
            "agc get-task <id> --fields status.state",
            "agc list-tasks --status working",
            "agc subscribe <id>       # stream live updates"
        ]
    })
}

fn card_schema() -> serde_json::Value {
    json!({
        "description": "AgentCard declares what an agent can do. Fetched via `agc card`.",
        "fields": {
            "name": "Human-readable agent name.",
            "description": "What the agent does.",
            "version": "Agent version string.",
            "url": "Base URL of the agent.",
            "capabilities": {
                "streaming": "true if the agent supports `agc stream`",
                "push_notifications": "true if the agent supports push-config callbacks",
                "extended_agent_card": "true if an authenticated extended card is available"
            },
            "skills": {
                "type": "array",
                "description": "List of skills the agent exposes.",
                "item_fields": {
                    "id": "Skill identifier — use with `agc schema skill <id>`",
                    "name": "Skill display name",
                    "description": "What this skill does",
                    "input_modes": "Accepted MIME types (e.g. text/plain)",
                    "output_modes": "Returned MIME types",
                    "tags": "Categorisation tags",
                    "examples": "Example prompts that trigger this skill"
                }
            },
            "security_schemes": "OAuth/auth requirements. If present, run `agc auth login` first."
        },
        "examples": [
            "agc card",
            "agc card --agent prod",
            "agc card --fields name,skills,capabilities",
            "agc card --fields skills"
        ]
    })
}
