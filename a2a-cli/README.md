# a2a-protocol-cli

`a2a-protocol-cli` provides:

- the `a2a` command-line tool for interacting with A2A protocol agents
- the `a2a_cli` Rust library crate for programmatic use

Install the CLI with Cargo:

```bash
cargo install a2a-protocol-cli
```

Use the library from Rust:

```rust,no_run
use a2a_cli::Client;

#[tokio::main]
async fn main() -> a2a_cli::error::Result<()> {
    let client = Client::new("https://agent.example.com")?;
    let response = client.send("Hello, agent!").await?;
    println!("{response}");
    Ok(())
}
```

See the repository README for full CLI and release documentation.
