use clap::Subcommand;

use crate::config::{config_path, load};
use crate::error::Result;
use crate::printer::print_json;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show,
}

pub async fn run_config(cmd: &ConfigCommand) -> Result<()> {
    match cmd {
        ConfigCommand::Show => {
            let cfg = load()?;
            let path = config_path().ok();
            print_json(
                &serde_json::json!({
                    "path": path.map(|p| p.display().to_string()),
                    "current_agent": cfg.current_agent,
                    "agents": cfg.agents.keys().collect::<Vec<_>>(),
                }),
                None,
                false,
            )?;
        }
    }
    Ok(())
}
