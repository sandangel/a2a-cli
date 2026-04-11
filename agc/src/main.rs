use std::sync::Arc;
use clap::Parser;

use agc::cli::{Cli, Command};
use agc::commands::{
    agent::run_agent, auth::run_auth, config::run_config,
    generate_skills::run_generate_skills, schema::run_schema,
};
use agc::client::{resolve_all_targets, resolve_target};
use agc::printer::{print_agent_json, print_json};
use agc::runner::{is_streaming, run_streaming, run_to_value};
use agc::token_store::load_token;

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
        Command::Agent { command }      => return run_agent(command).await,
        Command::Auth { command }       => return run_auth(command, args).await,
        Command::Config { command }     => return run_config(command).await,
        Command::GenerateSkills(cmd)    => return run_generate_skills(cmd).await,
        Command::Schema { command }     => return run_schema(command, args).await,
        _ => {}
    }

    let fields = args.fields.as_deref();
    let compact = args.compact;
    let command = Arc::new(cli.command);

    // Multi-agent: --all
    if args.all {
        let targets = resolve_all_targets()?;
        let mut handles = vec![];
        for t in targets {
            // Prefer explicit --bearer-token, fall back to stored token.
            let bearer = args.bearer_token.clone()
                .or_else(|| load_token(&t.url).ok().flatten().map(|tok| tok.access_token));
            let binding = args.binding;
            let tenant  = args.tenant.clone();
            let cmd     = Arc::clone(&command);
            handles.push(tokio::spawn(async move {
                run_to_value(&cmd, &t.url, bearer.as_deref(), binding, tenant.as_deref())
                    .await
                    .map(|v| (t.alias, t.url, v))
            }));
        }
        for h in handles {
            match h.await {
                Ok(Ok((alias, url, v))) => print_agent_json(&alias, &url, &v, fields)?,
                Ok(Err(e))              => eprintln!("error: {e}"),
                Err(e)                  => eprintln!("task error: {e}"),
            }
        }
        return Ok(());
    }

    // Single agent — prefer explicit --bearer-token, fall back to stored token.
    let target  = resolve_target(args)?;
    let stored  = load_token(&target.url).ok().flatten();
    let bearer: Option<String> = args.bearer_token.clone()
        .or_else(|| stored.map(|tok| tok.access_token));
    let bearer  = bearer.as_deref();
    let binding = args.binding;
    let tenant  = args.tenant.as_deref();

    if is_streaming(&command) {
        return run_streaming(&command, &target.url, bearer, binding, tenant, |v| {
            print_json(&v, fields, true) // streaming always compact (NDJSON)
        })
        .await;
    }

    let value = run_to_value(&command, &target.url, bearer, binding, tenant).await?;
    print_json(&value, fields, compact)
}
