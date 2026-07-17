![gre](https://raw.githubusercontent.com/tappunk/.github/refs/heads/main/assets/gre.webp)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Crates.io Version](https://img.shields.io/crates/v/gre?color=orange&cacheSeconds=3600)](https://crates.io/crates/gre)
[![GitHub Release](https://img.shields.io/github/v/release/tappunk/gre?color=blue)](https://github.com/tappunk/gre/releases)
[![X Follow](https://img.shields.io/twitter/follow/tappunk?style=social)](https://x.com/tappunk)

# gre

**Parallel git status aggregator for multiple repositories.** Human and JSON output for agent automation.

[Installation](#installation) • [Quick Start](#quick-start) • [Usage](#usage) • [Config](#config) • [Output Formats](#output-formats)

## Features

- **Parallel inspection** — rayon-powered concurrent repo scanning across all configured repositories
- **Action-aware sorting** — repos sorted by priority: conflicts → dirty → behind → ahead → clean
- **Focus line** — shows actionable repos and how many more need attention
- **Machine readable** — stable JSON output with timing stats (elapsed, avg, p95) for automation pipelines
- **Non-interactive** — no prompts, no TUI. Just output. Built for speed and scripts.
- **Configurable** — TOML config with simple path lists or named repo entries
- **Shell agnostic** — works in any terminal, pipe JSON to agents or other tools

## Installation

### Homebrew

```bash
brew install tappunk/gre/gre
```

### Cargo

```bash
cargo install gre
```

## Quick Start

```bash
gre init                     # Create config file
gre                          # Show status across all configured repos
```

## Usage

```bash
gre                          # Show human-readable status
gre --json                   # Output JSON for agents and scripts
gre --config PATH            # Use an explicit config file
gre --help                   # Show help
gre init                     # Create default config
gre init --force             # Overwrite existing config
```

## Config

Default path: `~/.config/gre/config.toml`

### Simple — just paths

```toml
paths = [
  "~/src/gre",
  "~/src/muthr",
  "~/src/utmd",
]
```

### Named — when you want a different label than the directory

```toml
[[repos]]
name = "gre"
path = "~/src/gre"

[[repos]]
name = "muthr"
path = "~/src/muthr"
```

## Output Formats

### Human Output

gre sorts repos by action priority: conflicts → dirty → behind → ahead → clean. The summary line includes timing (elapsed, average) and a focus line showing which repos need attention.

```
repos:3  dirty:1  behind:1  ahead:1  time:12ms  avg:4.00ms  focus:gre:commit-or-stash,muthr:pull,utmd:none
repo  branch  +sync  state  next  last_commit  path
gre   main     +1 -0  dirty s:0 u:2 ?:0 c:0  commit-or-stash  8a3b2f1 tune output (2 hours ago)  ~/src/gre
```

**Next action values:**

- `resolve-conflicts` — has merge conflicts
- `commit-or-stash` — has unstaged or untracked changes
- `pull` — behind remote
- `push` — ahead of remote
- `sync` — ahead and behind
- `none` — clean

### JSON Output

`--json` returns a stable schema for automation and agent pipelines:

```json
{
  "schema_version": "2",
  "summary": {
    "configured_total": 2,
    "succeeded_total": 2,
    "failed_total": 0,
    "dirty": 1,
    "behind": 0,
    "ahead": 1,
    "elapsed_ms": 42,
    "avg_repo_ms": 21.0,
    "p95_repo_ms": 38.2
  },
  "repos": [
    {
      "name": "gre",
      "path": "/Users/user/src/gre",
      "branch": "main",
      "ahead": 1,
      "behind": 0,
      "staged": 0,
      "unstaged": 2,
      "untracked": 0,
      "conflicts": 0,
      "clean": false,
      "last_hash": "8a3b2f1",
      "last_subject": "tune output",
      "last_relative": "2 hours ago"
    }
  ]
}
```

`configured_total` equals `succeeded_total + failed_total`. Repo fields include `path`, `clean`, `last_hash`, `last_subject`, and `last_relative`.

**Note:** gre does not fetch remotes. Run `git fetch` in your repos before running gre to refresh tracking refs.
