# Install and Rust Usage

This file covers install paths and Rust usage that are not part of the main
quick start. The Rust package is `a2a-protocol-cli`, and the library crate is
`a2a_cli`.

## npm Dist-Tags

The main README shows the stable npm install path. The default package install
uses the `latest` dist-tag. Release candidates are published with the `next`
tag, and builds from the `main` branch are published with the `dev` tag:

```bash
# Stable
npm install -g @sandangel/a2a-protocol-cli --registry=https://npm.pkg.github.com

# Release candidate
npm install -g @sandangel/a2a-protocol-cli@next --registry=https://npm.pkg.github.com

# Current main-branch build
npm install -g @sandangel/a2a-protocol-cli@dev --registry=https://npm.pkg.github.com
```

## Cargo

Install the CLI directly from the repository:

```bash
cargo install --git https://github.com/sandangel/a2a-cli a2a-protocol-cli
```

## Direct Download

Download a pre-built binary from the [Releases](https://github.com/sandangel/a2a-cli/releases) page.

```bash
# Example: Linux x86_64
curl -sLO https://github.com/sandangel/a2a-cli/releases/latest/download/a2a-x86_64-unknown-linux-gnu.tar.gz
tar -xzf a2a-x86_64-unknown-linux-gnu.tar.gz
chmod +x a2a && sudo mv a2a /usr/local/bin/
```

## Build From Source

Requires Rust 1.85+.

```bash
git clone https://github.com/sandangel/a2a-cli.git
cd a2a-cli
cargo build -p a2a-protocol-cli --release
# binary: target/release/a2a
```

## Shell Completions

Generate shell completion scripts with `a2a completions <shell>`.

```bash
# bash — add to ~/.bashrc
source <(a2a completions bash)

# zsh — add to ~/.zshrc
mkdir -p ~/.zsh/completions
a2a completions zsh > ~/.zsh/completions/_a2a
# then add to ~/.zshrc (if not already present):
#   fpath=(~/.zsh/completions $fpath)
#   autoload -Uz compinit && compinit

# fish
a2a completions fish > ~/.config/fish/completions/a2a.fish
```

## Rust API

Use the Rust crate directly when embedding A2A client behavior in another Rust
application:

```bash
cargo add a2a-protocol-cli --git https://github.com/sandangel/a2a-cli
```

```rust
use a2a_cli::{Client, SendOptions};

#[tokio::main]
async fn main() -> a2a_cli::error::Result<()> {
    let client = Client::new("https://agent.example.com")?;

    let response = client
        .send_with(
            "What is 2+2?",
            SendOptions::default().accept_output("text/plain"),
        )
        .await?;

    println!("{response}");
    Ok(())
}
```
