use clap::Subcommand;

use crate::cli::GlobalArgs;
use crate::config::{config_path, load};
use crate::error::{AgcError, Result};
use crate::printer::print_value;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show,
    /// Open the config file in $EDITOR (default: vi)
    Edit,
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
        ConfigCommand::Edit => {
            let path = config_path()?;
            // Create an empty config file if it doesn't exist yet.
            if !path.exists() {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(AgcError::Io)?;
                }
                std::fs::write(&path, "").map_err(AgcError::Io)?;
            }
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let status = std::process::Command::new(&editor)
                .arg(&path)
                .status()
                .map_err(|e| AgcError::Config(format!("failed to launch {editor:?}: {e}")))?;
            if !status.success() {
                return Err(AgcError::Config(format!(
                    "{editor} exited with status {status}"
                )));
            }
        }
    }
    Ok(())
}
