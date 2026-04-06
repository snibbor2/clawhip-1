# Filesystem-Offloaded Memory Guide

This guide shows how agents and operators should use the offloaded memory pattern in practice.

For the architecture/spec, see [Filesystem-Offloaded Memory Architecture](memory-offload-architecture.md).

## Operating rule

Treat `MEMORY.md` as the fast pointer layer, not the place where all detail accumulates.

A good default policy is:

- read `MEMORY.md` first
- jump to the smallest relevant shard
- write detail into leaf files
- update root pointers only when the map or current beliefs change

## Bootstrap with clawhip

clawhip now ships a first runtime vertical slice for this pattern:

```bash
# initialize a scaffold in the current repo
clawhip memory init --project clawhip --channel discord-alerts --agent codex

# pin a specific daily shard name when backfilling or scripting
clawhip memory init --project clawhip --date 2026-03-10

# inspect the scaffold and list missing recommended paths
clawhip memory status --project clawhip --channel discord-alerts --agent codex
```

What `clawhip memory init` bootstraps:

- `MEMORY.md`
- `memory/README.md`
- `memory/scaffold.toml`
- `memory/projects/<project>/README.md`
- `memory/projects/<project>/daily/YYYY-MM-DD.md`
- `memory/projects/<project>/audit/README.md`
- compatibility pointers such as `memory/daily/YYYY-MM-DD.md`
- compatibility pointers such as `memory/projects/<project>.md`
- `memory/topics/rules.md`
- `memory/topics/lessons.md`
- optional `memory/projects/<project>/channels/<channel>.md`
- optional `memory/channels/<channel>.md`
- optional `memory/agents/<agent>.md`

The command leaves existing files untouched by default and only overwrites scaffold files when you pass `--force`.

### Deep hierarchy mode

Use `--hierarchy deep` for the production-validated nested structure:

```bash
# full deep scaffold with folder-based daily partitions and tag headers
clawhip memory init --project clawhip --hierarchy deep --daily-format folder --tags
```

This creates the extended tree:

```
memory/
├── daily/
│   └── YYYY-MM/
│       └── DD/
│           ├── {project}.md
│           ├── heartbeat.md
│           ├── lessons.md
│           └── directives.md
├── projects/
│   └── {project}/
│       ├── plans/
│       ├── decisions/log.md
│       ├── status/current.md
│       └── reference/
├── ops/
│   ├── infra/
│   ├── rules/
│   └── sns/
├── channels/
│   ├── internal/
│   └── external/
├── bots/
├── bounties/
│   ├── active/
│   ├── prompts/
│   └── archive/
├── research/
│   ├── articles/
│   ├── proposals/
│   └── topics/
└── lessons.md
```

Flags:
- `--hierarchy flat` (default): backward-compatible flat scaffold
- `--hierarchy deep`: full nested tree
- `--daily-format file` (default): one `.md` per day
- `--daily-format folder`: one folder per day with category sub-files
- `--tags`: include Korean tag headers (`> 태그: #project #type`) in generated files

## Audit command

Check memory scaffold health:

```bash
# report issues
clawhip memory audit --project clawhip

# auto-fix detected issues
clawhip memory audit --project clawhip --fix

# send audit summary to Discord
clawhip memory audit --project clawhip --report-channel 1489922370063040522
```

Checks performed:
- Stray daily files at root level (should be in `daily/YYYY-MM/DD/`)
- Missing tag headers on markdown files
- `MEMORY.md` staleness (pointer count vs actual file count)
- Empty directories in the expected tree
- Projects missing `status/current.md` or `decisions/log.md`

## Daily rotation

Create today's daily folder with standard category stubs:

```bash
clawhip memory rotate --project clawhip
clawhip memory rotate --project clawhip --date 2026-04-06
```

## Cron audit

The first runtime slice also adds a scheduled scaffold audit via `[[cron.jobs]]`:

```toml
[[cron.jobs]]
id = "memory-audit"
schedule = "0 */6 * * *"
channel = "ops"
kind = "memory-audit"
root = "/path/to/repo"
project = "clawhip"
memory_channel = "discord-alerts"
auto_fix = true
```

When this job runs, clawhip:

- inspects the expected project/channel/daily partition files
- emits a custom event summarizing whether the scaffold looks ready
- appends a markdown note to `memory/projects/<project>/audit/cron/YYYY-MM-DD.md`

## What goes where

### Put in `MEMORY.md`

- active focus
- short current-state summary
- mandatory read paths for common situations
- write obligations
- links/pointers to canonical files
- recently moved or split sections

### Put in `memory/` leaf files

- detailed notes
- chronological logs
- channel-specific context
- project-specific state
- lessons, decisions, and operating rules
- handoff detail
- raw or semi-raw material that is too large for the hot layer

## Practical agent workflow

### Before acting

1. Read `MEMORY.md`.
2. Follow the scenario pointer for the current task.
3. Load only the relevant project/channel/topic/daily shard.
4. If no canonical target exists, create one in the correct subtree.

### While working

- append execution detail to the leaf shard that owns it
- keep root updates short and intentional
- if you discover a repeated retrieval path, add it to an index
- if a shard starts mixing unrelated topics, split it

### After working

- write detailed outcome to the canonical shard
- update `MEMORY.md` with only the new current belief or pointer change
- move stale time-based material to `archive/` when needed

## Recommended write-routing rules

Use rules like these:

| If the update is about... | Write to... |
|---|---|
| what happened today | `memory/projects/<project>/daily/YYYY-MM-DD.md` |
| one Discord/Slack/channel lane | `memory/projects/<project>/channels/<channel>.md` |
| one project/repo | `memory/projects/<project>/README.md` |
| one agent/operator profile | `memory/agents/<agent>.md` |
| reusable lessons | `memory/topics/lessons.md` |
| durable policies/rules | `memory/topics/rules.md` |
| one handoff | `memory/handoffs/YYYY-MM-DD-<slug>.md` |
| older inactive history | `memory/archive/...` |

## Migration: monolithic `MEMORY.md` -> offloaded memory

A safe migration path:

### 1. Freeze the role of `MEMORY.md`

Rewrite the file so it becomes:

- current beliefs
- file map
- scenario-based read guide
- write obligations

Do **not** keep adding detailed narrative after this step.

### 2. Identify high-growth sections

Typical sections to extract first:

- daily logs
- per-project sections
- per-channel sections
- long decision histories
- raw handoff dumps
- reusable rules/lessons hidden inside narrative blocks

### 3. Create the first shards

Start with the highest-leverage set:

```text
memory/README.md
memory/daily/
memory/projects/
memory/channels/
memory/topics/
memory/archive/
```

You do not need every subtree on day one.

### 4. Move detail, leave pointers

For each extracted section:

- move the detailed content into the new shard
- replace it in `MEMORY.md` with:
  - a short summary
  - the canonical file path
  - when to read it

### 5. Add write obligations

Make the system self-maintaining by stating rules such as:

- daily activity must go to today's daily file
- channel-specific context must go to the channel file
- durable lessons must be lifted into `topics/lessons.md`
- root memory must only hold summaries and pointers

### 6. Archive aggressively

Once a daily or project shard is no longer hot:

- compress it into a monthly archive bucket, or
- leave a short status summary and move the history out

## Refactor triggers

Refactor memory when:

- `MEMORY.md` stops being skimmable
- the same topic keeps expanding in the root file
- agents repeatedly read too much irrelevant context
- a file serves more than one clear owner
- retrieval depends on remembering ad hoc prose instead of stable paths

## Example starter set

Concrete example files in this repo:

- [docs/examples/MEMORY.example.md](examples/MEMORY.example.md)
- [docs/examples/memory/README.example.md](examples/memory/README.example.md)
- [docs/examples/memory/channels/example-channel.md](examples/memory/channels/example-channel.md)
- [docs/examples/memory/daily/2026-03-10.md](examples/memory/daily/2026-03-10.md)
- [skills/memory-offload/SKILL.md](../skills/memory-offload/SKILL.md)
- [docs/examples/memory/projects/clawhip.md](examples/memory/projects/clawhip.md)
- [docs/examples/memory/topics/rules.md](examples/memory/topics/rules.md)
- [docs/examples/memory/topics/lessons.md](examples/memory/topics/lessons.md)

## Cautions

- Do not turn the new tree into a second monolith.
- Do not create shards without clear read/write ownership.
- Do not expose sensitive memory in shared or automatically loaded files.
- Do not keep parallel daily-file conventions forever; pick one and normalize.
- Do not copy private production memory into public examples; abstract the pattern.

## Quick checklist

- Is `MEMORY.md` short and high-signal?
- Does every common workflow have a canonical file?
- Are daily logs separated from durable rules/lessons?
- Are archive rules clear?
- Can an agent tell where to write without guessing?
