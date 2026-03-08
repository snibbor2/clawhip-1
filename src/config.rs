use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::events::MessageFormat;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub routes: BTreeMap<String, RouteConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    pub bot_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    pub channel: Option<String>,
    #[serde(default)]
    pub format: MessageFormat,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            channel: None,
            format: MessageFormat::Compact,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteConfig {
    pub channel: Option<String>,
    pub format: Option<MessageFormat>,
    pub template: Option<String>,
}

pub fn default_config_path() -> PathBuf {
    if let Ok(override_path) = env::var("CLAWHIP_CONFIG") {
        return PathBuf::from(override_path);
    }

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".clawhip").join("config.toml")
}

impl AppConfig {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn to_pretty_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_pretty_toml()?)?;
        Ok(())
    }

    pub fn effective_token(&self) -> Option<String> {
        env::var("CLAWHIP_DISCORD_BOT_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| self.discord.bot_token.clone())
    }

    pub fn route(&self, key: &str) -> Option<&RouteConfig> {
        self.routes.get(key).or_else(|| {
            key.split_once('.')
                .and_then(|(prefix, _)| self.routes.get(prefix))
        })
    }

    pub fn run_interactive_editor(&mut self, path: &Path) -> Result<()> {
        println!("clawhip config editor");
        println!("Path: {}", path.display());
        println!();

        loop {
            self.print_summary();
            println!("Choose an action:");
            println!("  1) Set Discord bot token");
            println!("  2) Set default channel");
            println!("  3) Set default format");
            println!("  4) Add or update route");
            println!("  5) Remove route");
            println!("  6) Save and exit");
            println!("  7) Exit without saving");

            match prompt("Selection")?.trim() {
                "1" => self.discord.bot_token = empty_to_none(prompt("Bot token")?),
                "2" => self.defaults.channel = empty_to_none(prompt("Default channel")?),
                "3" => self.defaults.format = prompt_format(Some(self.defaults.format.clone()))?,
                "4" => self.upsert_route()?,
                "5" => self.remove_route()?,
                "6" => {
                    self.save(path)?;
                    println!("Saved {}", path.display());
                    break;
                }
                "7" => {
                    println!("Discarded changes.");
                    break;
                }
                _ => println!("Unknown selection."),
            }
            println!();
        }

        Ok(())
    }

    fn print_summary(&self) {
        let token_status = if self
            .discord
            .bot_token
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            "missing"
        } else {
            "configured"
        };
        println!("Current config summary:");
        println!("  Discord token: {token_status}");
        println!(
            "  Default channel: {}",
            self.defaults.channel.as_deref().unwrap_or("<unset>")
        );
        println!("  Default format: {}", self.defaults.format.as_str());
        if self.routes.is_empty() {
            println!("  Routes: <none>");
        } else {
            println!("  Routes:");
            for (name, route) in &self.routes {
                println!(
                    "    - {} => channel={}, format={}, template={}",
                    name,
                    route.channel.as_deref().unwrap_or("<default>"),
                    route
                        .format
                        .as_ref()
                        .map(MessageFormat::as_str)
                        .unwrap_or("<default>"),
                    route.template.as_deref().unwrap_or("<default>")
                );
            }
        }
        println!();
    }

    fn upsert_route(&mut self) -> Result<()> {
        let name = prompt("Route name (examples: custom, github.issue-opened, tmux.keyword)")?;
        let name = name.trim().to_string();
        if name.is_empty() {
            println!("Route name cannot be empty.");
            return Ok(());
        }

        let existing = self.routes.get(&name).cloned().unwrap_or_default();
        let channel = prompt_with_default("Route channel", existing.channel.as_deref())?;
        let format = prompt_format(existing.format.clone())?;
        let template = prompt_with_default("Route template", existing.template.as_deref())?;

        self.routes.insert(
            name,
            RouteConfig {
                channel: empty_to_none(channel),
                format: Some(format),
                template: empty_to_none(template),
            },
        );
        Ok(())
    }

    fn remove_route(&mut self) -> Result<()> {
        let name = prompt("Route name to remove")?;
        if self.routes.remove(name.trim()).is_some() {
            println!("Removed route {}", name.trim());
        } else {
            println!("No route named {}", name.trim());
        }
        Ok(())
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim_end().to_string())
}

fn prompt_with_default(label: &str, default: Option<&str>) -> Result<String> {
    let response = match default {
        Some(default) => prompt(&format!("{label} [{default}]"))?,
        None => prompt(label)?,
    };

    if response.trim().is_empty() {
        Ok(default.unwrap_or_default().to_string())
    } else {
        Ok(response)
    }
}

fn prompt_format(default: Option<MessageFormat>) -> Result<MessageFormat> {
    let default_value = default.unwrap_or(MessageFormat::Compact);
    let input = prompt(&format!(
        "Format [{}] (compact/alert/inline/raw)",
        default_value.as_str()
    ))?;
    if input.trim().is_empty() {
        return Ok(default_value);
    }
    MessageFormat::from_label(input.trim())
}

fn empty_to_none(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_lookup_falls_back_to_prefix() {
        let mut config = AppConfig::default();
        config.routes.insert(
            "github".to_string(),
            RouteConfig {
                channel: Some("123".into()),
                format: None,
                template: None,
            },
        );

        let route = config.route("github.issue-opened").unwrap();
        assert_eq!(route.channel.as_deref(), Some("123"));
    }
}
