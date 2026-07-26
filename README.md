# Codex Session Selector

A single Rust TUI executable for indexing, finding, and replaying local Codex
sessions.

Repository: <https://github.com/widehyo1/codex-session-selector.git>

https://github.com/user-attachments/assets/1885c129-d20a-4def-982f-3eb829745c11

## Binary

The package installs one executable:

- `select-codex-session`: opens the selector by default, builds the SQLite
  index with the `index` subcommand, and opens session timelines with the
  `replay` subcommand.

## Install

Building from source requires Rust 1.85 or newer.

```bash
cargo install codex-session-selector
```

From this repository:

```bash
./install-bundle.sh
```

The script installs to `~/.local/bin` by default. Override the destination with
`CODEX_CLI_BIN_DIR`:

```bash
CODEX_CLI_BIN_DIR=~/.cli/bin ./install-bundle.sh
```

Directly from Git:

```bash
cargo install --git https://github.com/widehyo1/codex-session-selector.git
```

## Quick Start

Open the selector:

```bash
select-codex-session
```

Before opening, it rebuilds `~/codex-session-info.sqlite3` from
`~/.codex/sessions`. Choose a session with `Enter`; when replay exits, the
selector returns with its selection, query, search scope, and focus intact.

Command-execution records are hidden and not indexed by default. To index them
and initially show legacy `exec_command_end` records plus newer
`exec`/`exec_command` tool calls and their matching outputs:

```bash
select-codex-session --include-exec
```

The selector and replay headers show `exec: hidden` or `exec: shown`. Press
`e` at runtime to change replay visibility without rebuilding the index.

## Selector

Useful options:

```bash
select-codex-session --no-refresh
select-codex-session --db /tmp/sessions.sqlite3
select-codex-session --print-path
```

Selector options:

```text
--db PATH
--record-command COMMAND
--replay-command COMMAND
--no-refresh
--include-exec
--print-path
-h, --help
-V, --version
```

`--record-command` and `--replay-command` are advanced compatibility
overrides. Without them, indexing and replay run inside the same executable.
An override value is treated as one executable path, not as a shell command.

### Selector controls

- `j/k` or `Up/Down`: move selection
- `d/u` or `PageDown/PageUp`: page selection or message scroll
- `g/G`: first/last selection or message top/bottom
- `Tab`: switch focus between sessions and first message
- `h/l` or `Left/Right`: horizontally scroll session metadata
- `0`: reset horizontal scroll
- `/`: interactive search
- `Tab` while searching: cycle `all`, `message`, `cwd`, `branch`, `repo`, `date`
- `e`: toggle exec visibility for the next replay
- `Enter`: replay the selected session, then return to the selector
- `y`: copy `codex resume <session-id>` to the clipboard
- `?`: show help
- `q`, `Esc`, or `Ctrl-C`: quit

Search is case-insensitive substring matching. All whitespace-separated terms
must match the selected scope. While searching, `e` is entered as query text
instead of toggling visibility.

An internal replay returns its final exec visibility to the selector, so the
next replay starts in the same state. An external `--replay-command` receives
the visibility that was active when it started, but visibility changes inside
that separate process are not returned to the selector.

## Index

Build or replace the default index:

```bash
select-codex-session index
```

Use custom paths, include normally filtered sessions, or index command
execution records:

```bash
select-codex-session index --output /tmp/sessions.sqlite3
select-codex-session index --sessions-root /path/to/.codex/sessions
select-codex-session index --include-subsessions --include-empty-messages
select-codex-session index --include-exec
```

Index options:

```text
-o, --output PATH
--sessions-root PATH
--include-subsessions
--include-empty-messages
--include-exec
-h, --help
-V, --version
```

Subsessions are excluded by default using `session_meta.payload`:
`source.subagent`, `thread_source == "subagent"`, or a non-empty `agent_role`.
Sessions without a first user message are also excluded by default.

## Replay

Replay a path directly:

```bash
select-codex-session replay ~/.codex/sessions/2026/07/26/rollout-....jsonl
select-codex-session replay --include-exec ~/.codex/sessions/2026/07/26/rollout-....jsonl
```

Raw JSONL and JSON arrays are also accepted on standard input. Both `-` and an
omitted path read stdin:

```bash
jq -c . session.jsonl | select-codex-session replay -
cat events.json | select-codex-session replay
```

Replay parses all supported user, agent, and command-execution entries once.
The initial state comes from `--include-exec`; pressing `e` then filters the
in-memory timeline immediately without rereading the JSONL.

### Replay controls

- `Tab`: switch focus between timeline and detail
- `j/k` or `Up/Down`: move through events or scroll detail
- `d/u` or `PageDown/PageUp`: move an event or scroll detail by a page
- `g/G` or `Home/End`: first/last event or detail top/bottom
- `1`: toggle timeline fullscreen
- `2`: toggle detail fullscreen
- `f`: toggle the focused pane fullscreen
- `e`: toggle command-execution entries
- `y`: copy the detail pane to the clipboard
- `?`: show help
- `q`, `Esc`, or `Ctrl-C`: quit

## Migration from 0.2

The standalone executables were replaced by subcommands:

```text
record-codex-session-info ARGS  → select-codex-session index ARGS
codex-replay-tui ARGS           → select-codex-session replay ARGS
```

Upgrading does not automatically delete previously installed standalone
executables. Remove old copies manually after confirming their install path.
The existing `~/codex-session-info.sqlite3` remains compatible.

## SQLite schema

The default index contains:

```text
sessions(path, id, timestamp, cwd, repository_url, branch, first_message)
```

With `select-codex-session index --include-exec`, it also contains:

```text
exec_events(
  session_path, session_id, event_index, call_id,
  kind, name, command, output
)
```

Indexing recreates `sessions`. Without `--include-exec`, `exec_events` is
removed; with it, that table is recreated and populated. Replay reads JSONL
directly rather than using `exec_events`. The TUI `e` toggle only changes the
in-memory replay view; it does not rebuild SQLite or add/remove tables.

The index still uses a full rebuild. A canonical, incremental index is not part
of this visibility feature and has not been implemented yet.

## Development

Enable the tracked pre-commit hook once per clone:

```bash
scripts/install-git-hooks.sh
```

The hook and GitHub Actions both call:

```bash
scripts/check-before-commit.sh
```

It checks formatting, Clippy with warnings denied, and all targets/features.
Commits are expected to remain green.
