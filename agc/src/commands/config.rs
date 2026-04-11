use clap::Subcommand;

use crate::cli::GlobalArgs;
use crate::config::{config_path, load};
use crate::error::Result;
use crate::printer::print_value;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show,
}

pub async fn run_config(cmd: &ConfigCommand, args: &GlobalArgs) -> Result<()> {
    match cmd {
        ConfigCommand::Show => {
            let cfg = load()?;
            let path = config_path().ok();
            print_value(
                &serde_json::json!({
                    "path": path.map(|p| p.display().to_string()),
                    "current_agent": cfg.current_agent,
                    "agents": cfg.agents.keys().collect::<Vec<_>>(),
                }),
                args.fields.as_deref(), args.format.clone(), args.compact,
            )?;
        }
    }
    Ok(())
}
