# Codex Session Selector

Rust TUI utilities for indexing, finding, and replaying local Codex sessions.

Repository: <https://github.com/widehyo1/codex-session-selector.git>

https://github.com/user-attachments/assets/1885c129-d20a-4def-982f-3eb829745c11

## Binaries

- `record-codex-session-info`: builds a SQLite index from local Codex session JSONL files.
- `select-codex-session`: searches the index in a TUI and opens the selected session in the replay TUI.
- `codex-replay-tui`: replays the user, assistant, and optionally command-execution timeline from a session JSONL file.

## Install

Building from source requires Rust 1.85 or newer.

Install all three binaries from crates.io:

```bash
cargo install codex-session-selector
```

Install from this repository:

```bash
./install-bundle.sh
```

By default, the script installs to `~/.local/bin`. Override the destination with:

```bash
CODEX_CLI_BIN_DIR=~/.cli/bin ./install-bundle.sh
```

Install directly from Git with:

```bash
cargo install --git https://github.com/widehyo1/codex-session-selector.git --bins
```

Manual install:

```bash
cargo build --release
install -m 755 target/release/record-codex-session-info ~/.local/bin/record-codex-session-info
install -m 755 target/release/select-codex-session ~/.local/bin/select-codex-session
install -m 755 target/release/codex-replay-tui ~/.local/bin/codex-replay-tui
```

## Quick Start

Open the session selector:

```bash
select-codex-session
```

The selector rebuilds `~/codex-session-info.sqlite3` from
`~/.codex/sessions` before opening. Choose a session with `Enter`; when replay
exits, the selector returns with its previous selection and search intact.

Command-execution records are hidden and not indexed by default. To index them
and show them in replay:

```bash
select-codex-session --include-exec
```

`--include-exec` is passed both to `record-codex-session-info` during refresh and
to `codex-replay-tui` after selecting a session. It supports legacy
`exec_command_end` records as well as `exec`/`exec_command` tool calls and their
matching outputs in newer JSONL files.

## Selector

Skip the automatic index refresh:

```bash
select-codex-session --no-refresh
```

Use a custom index or helper binary:

```bash
select-codex-session --db /tmp/sessions.sqlite3
select-codex-session --record-command /path/to/record-codex-session-info
select-codex-session --replay-command /path/to/codex-replay-tui
```

Select a session interactively and print its JSONL path instead of replaying it:

```bash
select-codex-session --print-path
```

Available options:

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

### Selector Controls

- `j/k` or `Up/Down`: move selection
- `d/u` or `PageDown/PageUp`: page selection or message scroll, depending on focus
- `g/G`: first/last selection or message top/bottom, depending on focus
- `Tab`: switch focus between sessions and first message
- `h/l` or `Left/Right`: horizontal scroll in the sessions pane
- `0`: reset horizontal scroll
- `/`: interactive search
- `Tab` while searching: cycle search scope through `all`, `message`, `cwd`, `branch`, `repo`, `date`
- `Enter`: run `codex-replay-tui` with the selected JSONL path, then return to the selector after replay exits
- `y`: copy `codex resume <session-id>` to the clipboard
- `?`: help
- `q`, `Esc`, or `Ctrl-C`: quit

Search terms are matched case-insensitively. Multiple whitespace-separated terms
must all match the selected scope.

## Recorder

Build or replace the default index manually:

```bash
record-codex-session-info
```

Use custom paths or include normally filtered sessions:

```bash
record-codex-session-info --output /tmp/sessions.sqlite3
record-codex-session-info --sessions-root /path/to/.codex/sessions
record-codex-session-info --include-subsessions --include-empty-messages
```

Add the command-execution index:

```bash
record-codex-session-info --include-exec
```

Recorder options:

```text
-o, --output PATH
--sessions-root PATH
--include-subsessions
--include-empty-messages
--include-exec
-h, --help
-V, --version
```

Subsessions are excluded by default using the `session_meta.payload` structure:
`source.subagent`, `thread_source == "subagent"`, or a non-empty `agent_role`.
Sessions without a first user message are also excluded by default.

## Replay

Replay a session directly:

```bash
codex-replay-tui ~/.codex/sessions/2026/07/26/rollout-....jsonl
codex-replay-tui --include-exec ~/.codex/sessions/2026/07/26/rollout-....jsonl
```

The replay TUI also accepts raw JSONL or a JSON array on standard input:

```bash
jq -c . session.jsonl | codex-replay-tui -
```

Replay options:

```text
--include-exec
-h, --help
-V, --version
```

### Replay Controls

- `Tab`: switch focus between timeline and detail
- `j/k` or `Up/Down`: move through events or scroll detail, depending on focus
- `d/u` or `PageDown/PageUp`: move one event or scroll detail by a page
- `g/G` or `Home/End`: first/last event or detail top/bottom
- `1`: toggle timeline fullscreen
- `2`: toggle detail fullscreen
- `f`: toggle the focused pane fullscreen
- `y`: copy the detail pane to the clipboard
- `?`: help
- `q`, `Esc`, or `Ctrl-C`: quit

## SQLite Schema

The default index contains:

```text
sessions(path, id, timestamp, cwd, repository_url, branch, first_message)
```

With `record-codex-session-info --include-exec`, it also contains:

```text
exec_events(
  session_path, session_id, event_index, call_id,
  kind, name, command, output
)
```

Running the recorder recreates the `sessions` table. Without `--include-exec`,
the `exec_events` table is removed; with it, the table is recreated and
populated. Replay reads the selected JSONL file directly; `exec_events` is
available to SQLite queries rather than being used as the replay source.
