# Codex Session Selector

Rust TUI utilities for indexing local Codex session JSONL files and selecting one to replay.

Repository: <https://github.com/widehyo1/codex-session-selector.git>

## Binaries

- `record-codex-session-info`: scans `~/.codex/sessions/*/*/*/*.jsonl` and writes a SQLite index.
- `select-codex-session`: opens a ratatui selector, refreshes the SQLite index by default, and runs `codex-replay-tui <jsonl-path>` for the selected session.
- `codex-replay-tui`: opens a ratatui replay view for one Codex session JSONL file.

## Install

Install all three binaries from crates.io:

```bash
cargo install codex-session-selector
```

Or install from this repository:

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

## Usage

Refresh the SQLite index manually:

```bash
record-codex-session-info
```

Open the selector:

```bash
select-codex-session
```

By default, `select-codex-session` runs `record-codex-session-info --output ~/codex-session-info.sqlite3` before opening the TUI. To skip refresh:

```bash
select-codex-session --no-refresh
```

Use a custom database or command:

```bash
select-codex-session --db /tmp/sessions.sqlite3
select-codex-session --record-command /path/to/record-codex-session-info
select-codex-session --replay-command /path/to/codex-replay-tui
```

## TUI Controls

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

When replay exits, the selector opens again with the previous selection and filter state intact.

## SQLite Schema

The `sessions` table contains:

- `path`
- `id`
- `timestamp`
- `cwd`
- `repository_url`
- `branch`
- `first_message`

Subsessions are excluded by default using `session_meta.payload` structure (`source.subagent`, `thread_source == "subagent"`, or `agent_role`). Empty first messages are also excluded by default.

To include them:

```bash
record-codex-session-info --include-subsessions --include-empty-messages
```
