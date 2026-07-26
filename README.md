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

Building from source requires Rust 1.97 or newer.

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

Before opening, it incrementally refreshes `~/codex-session-info.sqlite3` from
`~/.codex/sessions`. Choose a session with `Enter`; when replay exits, the
selector returns with its selection, query, search scope, and focus intact.

The canonical index always stores command-execution records. They remain hidden
in replay by default. To initially show legacy `exec_command_end` records plus
newer `exec`/`exec_command` tool calls and their matching outputs:

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
select-codex-session --include-subsessions
select-codex-session --include-empty-messages
```

Selector options:

```text
--db PATH
--record-command COMMAND
--replay-command COMMAND
--no-refresh
--include-subsessions
--include-empty-messages
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
- `Tab` while searching: cycle `all`, `message`, `cwd`, `branch`, `repo`, `date`, `exec`
- `e`: toggle exec visibility for the next replay
- `Enter`: replay the selected session, then return to the selector
- `y`: copy `codex resume <session-id>` to the clipboard
- `?`: show help
- `q`, `Esc`, or `Ctrl-C`: quit

Search uses the persistent FTS5 index and BM25 relevance. Bare
whitespace-separated terms are combined with AND and match token prefixes:
`fix read` finds “Fix README parser”, while the middle substring `ead` does not.
Double quotes select an exact token phrase, and a whitespace-delimited `|`
separates OR groups:

```text
"readme parser" | cargo test
```

The scopes search all indexed fields, the first message, cwd, branch,
repository URL, date, or command/output text respectively. FTS uses the
Unicode `unicode61` tokenizer. A Korean word without spaces is one token, so
`검색` finds `검색기능`, but the middle of that token (`기능`) does not.
While searching, `e` is entered as query text instead of toggling visibility.
Exec command and output are always searchable in `all` and `exec`; the
`exec: shown|hidden` state only controls the next replay.

An internal replay returns its final exec visibility to the selector, so the
next replay starts in the same state. An external `--replay-command` receives
the visibility that was active when it started, but visibility changes inside
that separate process are not returned to the selector.

The selector hides subsessions and sessions without a non-empty first message
by default, matching earlier releases. `--include-subsessions` and
`--include-empty-messages` expose those rows from the canonical index without
rebuilding or changing its stored contents.

## Index

Refresh the default index:

```bash
select-codex-session index
```

Use custom paths or force a full rebuild:

```bash
select-codex-session index --output /tmp/sessions.sqlite3
select-codex-session index --sessions-root /path/to/.codex/sessions
select-codex-session index --rebuild
```

Index options:

```text
-o, --output PATH
--sessions-root PATH
--rebuild
--include-subsessions
--include-empty-messages
--include-exec
-h, --help
-V, --version
```

The three `index --include-*` options remain accepted for script compatibility,
but they are no-ops: the canonical index always stores subsessions, empty
messages, and exec events. Use the corresponding root selector options to
change the visible session set. Subsessions are recognized using
`session_meta.payload.source.subagent`, `thread_source == "subagent"`, or a
non-empty `agent_role`.

Refresh uses each JSONL path, size, and modification timestamp. Unchanged files
are not opened or parsed; new and changed files alone are parsed, and rows for
deleted files are removed in the same transaction. A file that changes during
both parse attempts is deferred until the next refresh. Content rewritten to
the same size with its timestamp restored cannot be detected by this
fingerprint; `index --rebuild` is the recovery path.

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
The next refresh transactionally rebuilds an existing seven-column legacy
database from source JSONL. Schema v1 canonical indexes migrate in place to
schema v2 without reparsing unchanged JSONL. `--no-refresh` requires schema v2;
for an older index, refresh normally or run `select-codex-session index`.
Changing `--sessions-root` for an existing canonical database also causes an
automatic full rebuild. A newer schema version is never overwritten; an
unknown schema requires explicit `index --rebuild`.

## SQLite schema

The crate uses `rusqlite 0.40.1` with bundled SQLite `3.53.2`.

The index uses `PRAGMA user_version = 2`. Its canonical data remains in four
STRICT tables:

```text
index_metadata(singleton, sessions_root)
source_files(
  source_key, path, file_size, modified_secs, modified_nanos, parse_status
)
sessions(
  path, id, timestamp, cwd, repository_url, branch, first_message,
  session_key, source_key, is_subsession, has_nonempty_first_message
)
exec_events(
  session_path, session_id, event_index, call_id,
  kind, name, command, output, exec_key, session_key
)
```

`source_key` is stable for an unchanged source path, `session_key` is stable for
that source, and `exec_key` is stable for the same session and JSONL
`event_index` across incremental updates. File rename, root change, legacy
migration, and forced rebuild may assign new keys. Foreign keys cascade source
deletions through sessions and exec events.

All writes, including legacy migration and full rebuild, use one immediate
transaction and validate foreign keys and SQLite integrity before commit.
`sessions` and `exec_events` always exist. Replay still reads JSONL directly;
the TUI `e` toggle only changes the in-memory replay view.

Each canonical session also has one Contentless-Delete FTS5 document whose
`sessions_fts.rowid` is the stable `sessions.session_key`. It indexes the first
message, cwd, repository URL, branch, timestamp/date, and newline-joined exec
commands and outputs. The virtual table uses `unicode61 remove_diacritics 2`,
2/3-character prefix indexes, full phrase detail, and BM25 column sizes.

`fts_sync_state` and six mutation triggers detect canonical writes made outside
the indexer. A normal incremental refresh updates only touched FTS documents.
If the dirty state or FTS/canonical rowid sets disagree, the next refresh
rebuilds only the FTS objects while preserving canonical keys. For FTS shadow
corruption, use:

```bash
select-codex-session index --rebuild
```

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
