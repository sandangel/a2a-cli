use std::sync::Arc;
use clap::Parser;
use futures::stream::{FuturesUnordered, StreamExt};

use agc::cli::{Cli, Command};
use agc::commands::{
    agent::run_agent, auth::run_auth, config::run_config,
    generate_skills::run_generate_skills, schema::run_schema,
};
use agc::auth::refresh_if_expired;
use agc::client::{resolve_all_targets, resolve_target};
use agc::printer::{print_agent_json, print_json, print_value};
use agc::runner::{is_streaming, run_streaming, run_to_value};
use agc::validate::validate_message_text;

/// Resolve the bearer token: explicit flag takes priority, then stored token
/// (refreshing silently if the token is expired and a refresh_token is available).
async fn resolve_bearer(explicit: Option<String>, url: &str, client_id: &str) -> Option<String> {
    if explicit.is_some() {
        return explicit;
    }
    refresh_if_expired(url, client_id).await.ok().flatten()
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli).await {
        eprintln!("error: {e}");
        std::process::exit(e.exit_code());
    }
}

async fn dispatch(cli: Cli) -> agc::error::Result<()> {
    let args = &cli.global;

    // Management / tooling commands — no agent resolution needed.
    match &cli.command {
        Command::Agent { command }      => return run_agent(command, args).await,
        Command::Auth { command }       => return run_auth(command, args).await,
        Command::Config { command }     => return run_config(command, args).await,
        Command::GenerateSkills(cmd)    => return run_generate_skills(cmd).await,
        Command::Schema { command }     => return run_schema(command, args).await,
        _ => {}
    }

    // Validate message text for Send/Stream commands before any dispatch.
    match &cli.command {
        Command::Send(cmd) | Command::Stream(cmd) => validate_message_text(&cmd.text)?,
        _ => {}
    }

    let fields  = args.fields.as_deref();
    let compact = args.compact;
    let format  = args.format.clone();
    let command = Arc::new(cli.command);

    // Multi-agent: --all — dispatch in parallel, print results first-done-first.
    if args.all {
        let targets = resolve_all_targets()?;
        let futs: FuturesUnordered<_> = targets
            .into_iter()
            .map(|t| {
                let explicit_bearer = args.bearer_token.clone();
                let client_id = t.agent.oauth.client_id.clone();
                let binding = args.binding;
                let tenant  = args.tenant.clone();
                let cmd     = Arc::clone(&command);
                tokio::spawn(async move {
                    let bearer = resolve_bearer(explicit_bearer, &t.url, &client_id).await;
                    run_to_value(&cmd, &t.url, bearer.as_deref(), binding, tenant.as_deref())
                        .await
                        .map(|v| (t.alias, t.url, v))
                })
            })
            .collect();
        let mut stream = futs;
        while let Some(result) = stream.next().await {
            match result {
                Ok(Ok((alias, url, v))) => print_agent_json(&alias, &url, &v, fields)?,
                Ok(Err(e))              => eprintln!("error: {e}"),
                Err(e)                  => eprintln!("task error: {e}"),
            }
        }
        return Ok(());
    }

    // Single agent — prefer explicit --bearer-token, fall back to stored token (auto-refresh).
    let target = resolve_target(args)?;
    let bearer = resolve_bearer(args.bearer_token.clone(), &target.url, &target.agent.oauth.client_id).await;
    let bearer = bearer.as_deref();
    let binding = args.binding;
    let tenant  = args.tenant.as_deref();

    if is_streaming(&command) {
        return run_streaming(&command, &target.url, bearer, binding, tenant, |v| {
            print_json(&v, fields, true) // streaming always compact NDJSON
        })
        .await;
    }

    let value = run_to_value(&command, &target.url, bearer, binding, tenant).await?;
    print_value(&value, fields, format, compact)
}
