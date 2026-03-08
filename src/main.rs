mod cli;
mod config;
mod discord;
mod events;
mod router;
mod server;

use std::sync::Arc;

use clap::Parser;

use crate::cli::{Cli, Commands, ConfigCommand, GithubCommands, TmuxCommands};
use crate::config::AppConfig;
use crate::discord::DiscordClient;
use crate::events::IncomingEvent;
use crate::router::Router;

type DynError = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, DynError>;

#[tokio::main]
async fn main() {
    if let Err(error) = real_main().await {
        eprintln!("clawhip error: {error}");
        std::process::exit(1);
    }
}

async fn real_main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config_path();

    match cli.command {
        Commands::Config { command } => match command.unwrap_or(ConfigCommand::Interactive) {
            ConfigCommand::Interactive => {
                let mut config = AppConfig::load_or_default(&config_path)?;
                config.run_interactive_editor(&config_path)?;
            }
            ConfigCommand::Show => {
                let config = AppConfig::load_or_default(&config_path)?;
                println!("{}", config.to_pretty_toml()?);
            }
            ConfigCommand::Path => {
                println!("{}", config_path.display());
            }
        },
        command => {
            let config = Arc::new(AppConfig::load_or_default(&config_path)?);
            let router = Arc::new(Router::new(config.clone()));
            let discord = Arc::new(DiscordClient::from_config(config.clone())?);

            match command {
                Commands::Custom { channel, message } => {
                    let event = IncomingEvent::custom(channel, message);
                    router.dispatch(&event, discord.as_ref()).await?;
                }
                Commands::Github { command } => match command {
                    GithubCommands::IssueOpened {
                        repo,
                        number,
                        title,
                        channel,
                    } => {
                        let event =
                            IncomingEvent::github_issue_opened(repo, number, title, channel);
                        router.dispatch(&event, discord.as_ref()).await?;
                    }
                },
                Commands::Tmux { command } => match command {
                    TmuxCommands::Keyword {
                        session,
                        keyword,
                        line,
                        channel,
                    } => {
                        let event = IncomingEvent::tmux_keyword(session, keyword, line, channel);
                        router.dispatch(&event, discord.as_ref()).await?;
                    }
                },
                Commands::Stdin => {
                    let body = crate::events::read_stdin()?;
                    let events = crate::events::parse_stream(&body)?;
                    for event in events {
                        router.dispatch(&event, discord.as_ref()).await?;
                    }
                }
                Commands::Serve { port } => {
                    server::serve(([0, 0, 0, 0], port).into(), router, discord).await?;
                }
                Commands::Config { .. } => unreachable!("config handled earlier"),
            }
        }
    }

    Ok(())
}
