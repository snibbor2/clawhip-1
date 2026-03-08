<p align="center">
  <img src="assets/clawhip-mascot.jpg" alt="clawhip mascot" width="500">
</p>

<h1 align="center">🦞🔥 clawhip</h1>

<p align="center">
  <strong>claw + whip</strong> — standalone event-to-channel notification router<br>
  <em>The daemon that whips your clawdbot into shape.</em>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#install">Install</a> •
  <a href="#usage">Usage</a> •
  <a href="#config">Config</a>
</p>

---

## What is clawhip?

**clawhip** is a standalone notification router that takes events from CLI commands, stdin, and HTTP webhooks, then posts them directly to Discord channels through the Discord REST API.

It is an independent bot: **no clawdbot plugin, no Discord gateway integration, no shared session state**. You give it its own bot token, route rules, and output formats.

## Features

- 🔔 **Event routing** — `custom`, `github issue-opened`, `tmux keyword`, stdin JSON, and HTTP webhooks
- 💬 **Discord delivery** — Direct REST API via `reqwest`
- ⚙️ **CLI-first** — Single Rust binary with explicit subcommands
- 📋 **Flexible formats** — `compact`, `alert`, `inline`, `raw` per-route
- 🌐 **Webhook server** — `clawhip serve` exposes `/health`, `/events`, and `/github`
- 🛠️ **Interactive config editor** — `clawhip config` manages token/defaults/routes in `~/.clawhip/config.toml`

## Install

```bash
cargo install --path .
```

## Usage

```bash
# Send a custom notification
clawhip custom --channel 1468539002985644084 --message "Build complete! 🟢"

# GitHub issue-opened event
clawhip github issue-opened \
  --repo oh-my-claudecode \
  --number 1460 \
  --title "Bug in setup"

# tmux keyword detection
clawhip tmux keyword \
  --session issue-1440 \
  --keyword "PR created" \
  --line "PR #1453 merged"

# Pipe a single JSON event
printf '%s\n' '{"type":"custom","channel":"1468539002985644084","message":"Hello!"}' | clawhip stdin

# Pipe newline-delimited JSON events
printf '%s\n%s\n' \
  '{"type":"custom","message":"first"}' \
  '{"type":"custom","message":"second"}' | clawhip stdin

# Start the webhook server
clawhip serve --port 8765

# Interactive config editor
clawhip config

# Inspect config
clawhip config show
clawhip config path
```

## Config

Config lives at `~/.clawhip/config.toml`.

Example:

```toml
[discord]
bot_token = "your-discord-bot-token"

[defaults]
channel = "1468539002985644084"
format = "compact"

[routes.custom]
channel = "1468539002985644084"
format = "inline"

[routes."github.issue-opened"]
channel = "1468539002985644084"
format = "alert"
template = "🚨 {repo} #{number}: {title}"

[routes."tmux.keyword"]
format = "compact"
```

Environment overrides:

- `CLAWHIP_CONFIG` — override the config file path
- `CLAWHIP_DISCORD_BOT_TOKEN` — override the Discord bot token
- `CLAWHIP_DISCORD_API_BASE` — override the Discord API base URL (useful for tests)

### Route behavior

Route resolution precedence:

1. Explicit event `channel`
2. Matching route `channel`
3. Global default `defaults.channel`

Format resolution precedence:

1. Explicit event `format`
2. Matching route `format`
3. Global default `defaults.format`

Template resolution precedence:

1. Explicit event `template`
2. Matching route `template`
3. Built-in formatter for the event kind

### Message formats

| Format | Use Case | Example |
|--------|----------|---------|
| `compact` | Routine updates | `bellman/clawhip#17 opened: Webhook test` |
| `alert` | Failures / urgent events | `🚨 tmux session issue-1440 hit keyword 'panic': stack trace...` |
| `inline` | Dense channel output | `[GitHub] bellman/clawhip#17 Webhook test` |
| `raw` | Debugging / passthrough | Pretty-printed JSON payload |

## Event JSON

`clawhip stdin` and `POST /events` accept either a `payload` object or flat event-specific fields.

Flat example:

```json
{
  "type": "custom",
  "channel": "1468539002985644084",
  "message": "Deploy completed"
}
```

Payload example:

```json
{
  "type": "github.issue-opened",
  "payload": {
    "repo": "bellman/clawhip",
    "number": 17,
    "title": "Webhook test"
  }
}
```

## HTTP server

- `GET /health`
- `POST /events` — accepts the same event JSON as `stdin`
- `POST /github` — accepts GitHub `issues` webhook payloads and routes `action=opened` as `github.issue-opened`

## Architecture

```text
[CLI/stdin/webhooks] -> [event parsing] -> [route resolution + formatting] -> [Discord REST API] -> [channel]
```

No gateway. No clawdbot plugin boundary. Just events in and messages out.

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

## License

MIT
