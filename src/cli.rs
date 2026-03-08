use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "clawhip",
    version,
    about = "Standalone event-to-channel notification router for Discord"
)]
pub struct Cli {
    /// Override the config file path.
    #[arg(long, global = true, env = "CLAWHIP_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn config_path(&self) -> PathBuf {
        self.config
            .clone()
            .unwrap_or_else(crate::config::default_config_path)
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Send a custom notification.
    Custom {
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        message: String,
    },
    /// Emit GitHub-related notifications.
    Github {
        #[command(subcommand)]
        command: GithubCommands,
    },
    /// Emit tmux-related notifications.
    Tmux {
        #[command(subcommand)]
        command: TmuxCommands,
    },
    /// Read JSON event objects from stdin.
    Stdin,
    /// Run an HTTP webhook receiver.
    Serve {
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
    /// Manage configuration.
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },
}

#[derive(Debug, Subcommand)]
pub enum GithubCommands {
    /// Emit a GitHub issue-opened event.
    IssueOpened {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        number: u64,
        #[arg(long)]
        title: String,
        #[arg(long)]
        channel: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TmuxCommands {
    /// Emit a tmux keyword event.
    Keyword {
        #[arg(long)]
        session: String,
        #[arg(long)]
        keyword: String,
        #[arg(long)]
        line: String,
        #[arg(long)]
        channel: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Subcommand)]
pub enum ConfigCommand {
    /// Open the interactive config editor.
    #[default]
    Interactive,
    /// Print the active config as TOML.
    Show,
    /// Print the config file path.
    Path,
}
