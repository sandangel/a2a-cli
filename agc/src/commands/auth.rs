use clap::Subcommand;

use crate::auth::{TokenStatus, login, logout, token_status};
use crate::cli::GlobalArgs;
use crate::client::resolve_target;
use crate::config::load;
use crate::error::Result;
use crate::printer::print_value;
use crate::runner::fetch_card_raw;

fn status_json(alias: &str, url: &str, s: &TokenStatus) -> serde_json::Value {
    serde_json::json!({
        "alias": alias,
        "url": url,
        "authenticated": s.authenticated,
        "expired": s.expired,
        "expires_at": s.expires_at,
        "scopes": s.scopes,
        "access_token": s.masked_token,
    })
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Authenticate with an agent (auto-detects OAuth flow from agent card)
    Login,
    /// Remove stored credentials for an agent
    Logout,
    /// Show authentication status
    Status,
}

pub async fn run_auth(cmd: &AuthCommand, args: &GlobalArgs) -> Result<()> {
    match cmd {
        AuthCommand::Login => {
            let target = resolve_target(args)?;
            let card = fetch_card_raw(&target.url, args.bearer_token.as_deref()).await?;

            match login(&target.url, &target.agent, &card).await? {
                Some(_) => eprintln!("Authenticated with {:?} ({})", target.alias, target.url),
                None => eprintln!("Agent {} does not require authentication.", target.url),
            }
        }
        AuthCommand::Logout => {
            let target = resolve_target(args)?;
            logout(&target.url)?;
            eprintln!(
                "Credentials removed for {:?} ({})",
                target.alias, target.url
            );
        }
        AuthCommand::Status => {
            if !args.agent.is_empty() {
                let target = resolve_target(args)?;
                let s = token_status(&target.url)?;
                print_value(
                    &status_json(&target.alias, &target.url, &s),
                    args.fields.as_deref(),
                    args.format.clone(),
                    args.compact,
                )?;
            } else {
                let cfg = load()?;
                if cfg.agents.is_empty() {
                    eprintln!("No agents registered.");
                    return Ok(());
                }
                let mut statuses = vec![];
                for (alias, agent) in &cfg.agents {
                    let s = token_status(&agent.url)?;
                    statuses.push(status_json(alias, &agent.url, &s));
                }
                print_value(
                    &serde_json::Value::Array(statuses),
                    args.fields.as_deref(),
                    args.format.clone(),
                    args.compact,
                )?;
            }
        }
    }
    Ok(())
}
