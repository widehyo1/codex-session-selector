#!/usr/bin/env bash
set -euo pipefail

selector_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

bin_dir="${CODEX_CLI_BIN_DIR:-$HOME/.local/bin}"

mkdir -p "$bin_dir"

cargo build --release --manifest-path "$selector_dir/Cargo.toml"

install -m 755 "$selector_dir/target/release/record-codex-session-info" "$bin_dir/record-codex-session-info"
install -m 755 "$selector_dir/target/release/select-codex-session" "$bin_dir/select-codex-session"
install -m 755 "$selector_dir/target/release/codex-replay-tui" "$bin_dir/codex-replay-tui"

printf 'installed to %s\n' "$bin_dir"
