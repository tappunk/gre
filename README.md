![gre](https://raw.githubusercontent.com/tappunk/.github/refs/heads/main/assets/gre.webp)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Crates.io Version](https://img.shields.io/crates/v/gre?color=orange&cacheSeconds=3600)](https://crates.io/crates/gre)
[![GitHub Release](https://img.shields.io/github/v/release/tappunk/gre?color=blue)](https://github.com/tappunk/gre/releases)
[![X Follow](https://img.shields.io/twitter/follow/tappunk?style=social)](https://x.com/tappunk)

# gre

A super fast multi repo git recap for AI agents and speed obsessed humans.

Run one command:

```bash
gre
```

No prompts. No interactive mode. No extra setup after config.

## Install

```bash
cargo install gre
# or
brew install tappunk/gre/gre
```

## CLI

```text
gre [--config PATH] [--json] [init [--force]]
```

Minimal contract:

- `gre` prints the default human recap.
- `gre --json` prints machine-readable output for agents/scripts.
- `gre init` writes `~/.config/gre/config.toml`.
- `gre init --force` overwrites an existing config.
- `gre --config PATH` uses an explicit config path.

## Config

Default path:

```bash
~/.config/gre/config.toml
```

Example config:

```toml
paths = [
  "~/src/gre",
  "~/src/muthr",
]

[output]
default_json = false
```

Alternative repo shapes are also supported:

```toml
repos = ["~/src/gre", "~/src/muthr"]
```

```toml
[[repos]]
name = "gre"
path = "~/src/gre"
```

## Human Output

`gre` sorts repos by action priority: conflicts -> dirty -> behind -> ahead -> clean.

`ahead`/`behind` is local-tracking based. `gre` does not fetch remotes.
Run `git fetch` in a repo to refresh tracking refs before running `gre`.

The summary line includes run timing (`time`, `avg`) to track command speed.

Columns:

```text
repo  branch  sync  state  next  last_commit  path
```

`next` values:

- `resolve-conflicts`
- `commit-or-stash`
- `pull`
- `push`
- `sync`
- `none`

## JSON Output

`gre --json` is stable for automation and includes:

- `schema_version`
- `summary` counts (`configured_total`, `succeeded_total`, `failed_total`, `total`)
- `summary` repo state (`dirty`, `behind`, `ahead`)
- `summary` timing (`elapsed_ms`, `avg_repo_ms`)
- `summary` tail latency (`p95_repo_ms`)
- `repos` array

Example:

```json
{
  "schema_version": "1",
  "summary": {
    "total": 2,
    "dirty": 1,
    "behind": 0,
    "ahead": 1
  },
  "repos": [
    {
      "name": "gre",
      "path": "/Users/alice/src/gre",
      "branch": "main",
      "ahead": 1,
      "behind": 0,
      "staged": 0,
      "unstaged": 2,
      "untracked": 0,
      "conflicts": 0,
      "clean": false,
      "last_hash": "abcd123",
      "last_subject": "tune output",
      "last_relative": "2 hours ago"
    }
  ]
}
```
