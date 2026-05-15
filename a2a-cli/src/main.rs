use clap::Parser;
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;

use a2a_cli::auth::{refresh_if_expired, resolve_oauth_client_id};
use a2a_cli::cli::{Cli, Command};
use a2a_cli::client::{
    explicit_agent_refs, resolve_all_targets, resolve_explicit_targets, resolve_target,
};
use a2a_cli::commands::{
    agent::run_agent, auth::run_auth, config::run_config, generate_skills::run_generate_skills,
    schema::run_schema,
};
use a2a_cli::printer::{print_agent_json, print_json, print_value};
use a2a_cli::runner::{is_streaming, run_streaming, run_to_value_with_retry};
use a2a_cli::validate::validate_message_text;

/// Resolve the bearer token: explicit flag takes priority, then stored token
/// (refreshing silently when the stored token is expired and renewable).
async fn resolve_bearer(
    explicit: Option<String>,
    url: &str,
    client_id: &str,
) -> a2a_cli::error::Result<Option<String>> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    refresh_if_expired(url, client_id).await
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli).await {
        eprintln!("error: {e}");
        std::process::exit(e.exit_code());
    }
}

async fn dispatch(cli: Cli) -> a2a_cli::error::Result<()> {
    let args = &cli.global;

    // Management / tooling commands — no agent resolution needed.
    match &cli.command {
        Command::Agent { command } => return run_agent(command, args).await,
        Command::Auth { command } => return run_auth(command, args).await,
        Command::Config { command } => return run_config(command, args).await,
        Command::GenerateSkills(cmd) => return run_generate_skills(cmd).await,
        Command::Schema { command } => return run_schema(command, args).await,
        Command::Completions { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            generate(*shell, &mut cmd, bin_name, &mut std::io::stdout());
            return Ok(());
        }
        _ => {}
    }

    // Validate message text for Send/Stream commands before any dispatch.
    match &cli.command {
        Command::Send(cmd) | Command::Stream(cmd) => validate_message_text(&cmd.text)?,
        _ => {}
    }

    let fields = args.fields.as_deref();
    let compact = args.compact;
    let format = args.format.clone();
    let command = Arc::new(cli.command);

    let explicit_agents = if args.all {
        Vec::new()
    } else {
        explicit_agent_refs(args)?
    };

    // Multi-agent: --all or multiple explicit agent refs — dispatch in parallel.
    if args.all || explicit_agents.len() > 1 {
        let targets = if args.all {
            resolve_all_targets()?
        } else {
            resolve_explicit_targets(args)?
        };
        // In-process circuit breaker: tracks agent URLs that have tripped (permanent
        // failure after all retries).  If the same URL appears twice in the targets
        // list, the second call short-circuits immediately instead of burning retries.
        let tripped: Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
            Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

        let targets: Vec<_> = targets
            .into_iter()
            .map(|t| {
                let explicit_bearer = args.bearer_token();
                let client_id = if explicit_bearer.is_some() {
                    String::new()
                } else {
                    resolve_oauth_client_id(&t.agent, None)?.unwrap_or_default()
                };
                Ok((t, explicit_bearer, client_id))
            })
            .collect::<a2a_cli::error::Result<_>>()?;

        let futs: FuturesUnordered<_> = targets
            .into_iter()
            .map(|(t, explicit_bearer, client_id)| {
                let binding = args.transport;
                let tenant = args.tenant.clone();
                let cmd = Arc::clone(&command);
                let tripped = Arc::clone(&tripped);
                tokio::spawn(async move {
                    // Short-circuit if this URL already tripped in a parallel branch.
                    if tripped.lock().unwrap().contains(&t.url) {
                        return Err(a2a_cli::error::A2aCliError::Config(format!(
                            "circuit open for {} — previous call failed permanently",
                            t.url
                        )));
                    }
                    let bearer = resolve_bearer(explicit_bearer, &t.url, &client_id).await?;
                    let result = run_to_value_with_retry(
                        &cmd,
                        &t.url,
                        bearer.as_deref(),
                        binding,
                        tenant.as_deref(),
                    )
                    .await;
                    // Trip the circuit on permanent failure so duplicates skip immediately.
                    if let Err(ref e) = result
                        && !e.is_retryable()
                    {
                        tripped.lock().unwrap().insert(t.url.clone());
                    }
                    result.map(|v| (t.alias, t.url, v))
                })
            })
            .collect();
        let mut stream = futs;
        let mut any_success = false;
        let mut last_err: Option<a2a_cli::error::A2aCliError> = None;
        while let Some(result) = stream.next().await {
            match result {
                Ok(Ok((alias, url, v))) => {
                    print_agent_json(&alias, &url, &v, fields)?;
                    any_success = true;
                }
                Ok(Err(e)) => {
                    eprintln!("error: {e}");
                    last_err = Some(e);
                }
                Err(e) => eprintln!("task error: {e}"),
            }
        }
        // If every agent failed, propagate the last error so the process exits non-zero.
        if !any_success && let Some(e) = last_err {
            return Err(e);
        }
        return Ok(());
    }

    // Single agent — prefer explicit --bearer-token, fall back to stored token (auto-refresh).
    let target = resolve_target(args)?;
    let client_id = if args.bearer_token().is_some() {
        String::new()
    } else {
        resolve_oauth_client_id(&target.agent, None)?.unwrap_or_default()
    };
    let bearer = resolve_bearer(args.bearer_token(), &target.url, &client_id).await?;
    let bearer = bearer.as_deref();
    let binding = args.transport;
    let tenant = args.tenant.as_deref();

    if is_streaming(&command) {
        return tokio::select! {
            r = run_streaming(&command, &target.url, bearer, binding, tenant, |v| {
                print_json(&v, fields, true) // streaming always compact NDJSON
            }) => r,
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nInterrupted.");
                Ok(())
            }
        };
    }

    // Blocking commands: show a spinner on TTY so the user knows something is happening,
    // and cancel cleanly on Ctrl+C.
    let spinner = if matches!(
        *command,
        Command::Send(_) | Command::Card | Command::ExtendedCard
    ) {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.dim} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        pb.set_message("Waiting for response...");
        pb.enable_steady_tick(Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };

    let result = tokio::select! {
        r = run_to_value_with_retry(&command, &target.url, bearer, binding, tenant) => r,
        _ = tokio::signal::ctrl_c() => {
            if let Some(pb) = &spinner { pb.finish_and_clear(); }
            eprintln!("\nInterrupted.");
            return Ok(());
        }
    };

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    let value = result?;
    print_value(&value, fields, format, compact)
}
