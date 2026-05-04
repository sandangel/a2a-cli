//! Programmatic Rust API for the `a2a` client.
//!
//! The CLI and this API share the same request construction and protocol
//! compatibility path, so Rust callers get the same A2A v1/v0.3 handling as
//! the command-line tool without spawning a subprocess.

use serde_json::Value;

use crate::cli::Command;
use crate::commands::task::TaskCommand;
use crate::error::Result;
use crate::runner::{fetch_card, run_streaming, run_to_value, run_to_value_with_retry};
use crate::validate::{validate_agent_url, validate_message_text};

pub use a2acli::Binding as Transport;
pub use a2acli::TaskStateArg;

/// A reusable client for one A2A agent endpoint.
#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    bearer_token: Option<String>,
    transport: Option<Transport>,
    tenant: Option<String>,
    retry: bool,
}

impl Client {
    /// Create a client for an agent base URL.
    ///
    /// For more options, use [`Client::builder`].
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        Self::builder(base_url).build()
    }

    /// Start building a client for an agent base URL.
    pub fn builder(base_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(base_url)
    }

    /// Fetch the agent card as a typed A2A v1 card. v0.3 cards are normalized.
    pub async fn agent_card(&self) -> Result<a2a::AgentCard> {
        fetch_card(&self.base_url, self.bearer_token.as_deref()).await
    }

    /// Fetch the public agent card as JSON.
    pub async fn card(&self) -> Result<Value> {
        self.run(&Command::Card).await
    }

    /// Fetch the authenticated extended agent card as JSON.
    pub async fn extended_card(&self) -> Result<Value> {
        self.run(&Command::ExtendedCard).await
    }

    /// Send a text message with default send options.
    pub async fn send(&self, text: impl Into<String>) -> Result<Value> {
        self.send_with(text, SendOptions::default()).await
    }

    /// Send a text message with explicit send options.
    pub async fn send_with(&self, text: impl Into<String>, options: SendOptions) -> Result<Value> {
        let text = text.into();
        validate_message_text(&text)?;
        let command = Command::Send(options.into_message_command(text));
        self.run(&command).await
    }

    /// Send a streaming text message and handle each event as JSON.
    pub async fn stream(
        &self,
        text: impl Into<String>,
        on_event: impl FnMut(Value) -> Result<()>,
    ) -> Result<()> {
        self.stream_with(text, SendOptions::default(), on_event)
            .await
    }

    /// Send a streaming text message with explicit options.
    pub async fn stream_with(
        &self,
        text: impl Into<String>,
        options: SendOptions,
        on_event: impl FnMut(Value) -> Result<()>,
    ) -> Result<()> {
        let text = text.into();
        validate_message_text(&text)?;
        let command = Command::Stream(options.into_message_command(text));
        self.run_stream(&command, on_event).await
    }

    /// Fetch a task by ID.
    pub async fn task(&self, id: impl Into<String>) -> Result<Value> {
        self.task_with_history(id, None).await
    }

    /// Fetch a task by ID, optionally including history.
    pub async fn task_with_history(
        &self,
        id: impl Into<String>,
        history_length: Option<i32>,
    ) -> Result<Value> {
        let command = Command::Task {
            command: TaskCommand::Get(a2acli::TaskLookupCommand {
                id: id.into(),
                history_length,
            }),
        };
        self.run(&command).await
    }

    /// List tasks with optional filters.
    pub async fn list_tasks(&self, options: TaskListOptions) -> Result<Value> {
        let command = Command::Task {
            command: TaskCommand::List(a2acli::ListTasksCommand {
                context_id: options.context_id,
                status: options.status,
                page_size: options.page_size,
                page_token: options.page_token,
                history_length: options.history_length,
                include_artifacts: options.include_artifacts,
            }),
        };
        self.run(&command).await
    }

    /// Cancel a task by ID.
    pub async fn cancel_task(&self, id: impl Into<String>) -> Result<Value> {
        let command = Command::Task {
            command: TaskCommand::Cancel(a2acli::TaskIdCommand { id: id.into() }),
        };
        self.run(&command).await
    }

    /// Subscribe to task updates and handle each event as JSON.
    pub async fn subscribe_task(
        &self,
        id: impl Into<String>,
        on_event: impl FnMut(Value) -> Result<()>,
    ) -> Result<()> {
        let command = Command::Task {
            command: TaskCommand::Subscribe(a2acli::TaskIdCommand { id: id.into() }),
        };
        self.run_stream(&command, on_event).await
    }

    /// Run an advanced non-streaming command built from the CLI command model.
    pub async fn run(&self, command: &Command) -> Result<Value> {
        if self.retry {
            run_to_value_with_retry(
                command,
                &self.base_url,
                self.bearer_token.as_deref(),
                self.transport,
                self.tenant.as_deref(),
            )
            .await
        } else {
            run_to_value(
                command,
                &self.base_url,
                self.bearer_token.as_deref(),
                self.transport,
                self.tenant.as_deref(),
            )
            .await
        }
    }

    /// Run an advanced streaming command built from the CLI command model.
    pub async fn run_stream(
        &self,
        command: &Command,
        on_event: impl FnMut(Value) -> Result<()>,
    ) -> Result<()> {
        run_streaming(
            command,
            &self.base_url,
            self.bearer_token.as_deref(),
            self.transport,
            self.tenant.as_deref(),
            on_event,
        )
        .await
    }
}

/// Builder for [`Client`].
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    base_url: String,
    bearer_token: Option<String>,
    transport: Option<Transport>,
    tenant: Option<String>,
    retry: bool,
}

impl ClientBuilder {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            bearer_token: None,
            transport: None,
            tenant: None,
            retry: true,
        }
    }

    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    pub fn transport(mut self, transport: Transport) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    pub fn retry(mut self, retry: bool) -> Self {
        self.retry = retry;
        self
    }

    pub fn build(self) -> Result<Client> {
        validate_agent_url(&self.base_url)?;
        Ok(Client {
            base_url: self.base_url,
            bearer_token: self.bearer_token,
            transport: self.transport,
            tenant: self.tenant,
            retry: self.retry,
        })
    }
}

/// Options for `message/send` and `message/stream`.
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    pub context_id: Option<String>,
    pub task_id: Option<String>,
    pub history_length: Option<i32>,
    pub accepted_output_modes: Vec<String>,
    pub return_immediately: bool,
}

impl SendOptions {
    pub fn context_id(mut self, context_id: impl Into<String>) -> Self {
        self.context_id = Some(context_id.into());
        self
    }

    pub fn task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn history_length(mut self, history_length: i32) -> Self {
        self.history_length = Some(history_length);
        self
    }

    pub fn accept_output(mut self, mode: impl Into<String>) -> Self {
        self.accepted_output_modes.push(mode.into());
        self
    }

    pub fn return_immediately(mut self, return_immediately: bool) -> Self {
        self.return_immediately = return_immediately;
        self
    }

    fn into_message_command(self, text: String) -> a2acli::MessageCommand {
        a2acli::MessageCommand {
            text,
            context_id: self.context_id,
            task_id: self.task_id,
            history_length: self.history_length,
            accepted_output_modes: self.accepted_output_modes,
            return_immediately: self.return_immediately,
        }
    }
}

/// Options for task listing.
#[derive(Debug, Clone, Default)]
pub struct TaskListOptions {
    pub context_id: Option<String>,
    pub status: Option<TaskStateArg>,
    pub page_size: Option<i32>,
    pub page_token: Option<String>,
    pub history_length: Option<i32>,
    pub include_artifacts: bool,
}
