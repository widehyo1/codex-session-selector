#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

cd "$repo_root"

existing="$(git config --local --get core.hooksPath || true)"

if [[ -n "$existing" && "$existing" != ".githooks" ]]; then
  echo "core.hooksPath is already set to: $existing" >&2
  echo "refusing to overwrite the existing local hook path" >&2
  exit 1
fi

git config --local core.hooksPath .githooks

actual="$(git config --local --get core.hooksPath)"
if [[ "$actual" != ".githooks" ]]; then
  echo "failed to configure core.hooksPath" >&2
  exit 1
fi

echo "configured core.hooksPath=.githooks"
