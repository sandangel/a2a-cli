//! `agc task` subcommand — task lifecycle management.

use clap::Subcommand;

pub use a2acli::{ListTasksCommand, TaskIdCommand, TaskLookupCommand};

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// Fetch a task by ID
    Get(TaskLookupCommand),
    /// List tasks with optional filters
    List(ListTasksCommand),
    /// Cancel a running task
    Cancel(TaskIdCommand),
    /// Subscribe to live task updates (streaming)
    Subscribe(TaskIdCommand),
}
