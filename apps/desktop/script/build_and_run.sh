#!/usr/bin/env bash
# Full macOS rebuild/restart for local QA; release signing belongs to the release workflow.
set -euo pipefail
DESKTOP_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
VERIFY=false
if [[ "${1:-}" == "--verify" ]]; then VERIFY=true; elif [[ -n "${1:-}" ]]; then printf 'Usage: %s [--verify]\n' "$0" >&2; exit 2; fi
export PATH="$HOME/.cargo/bin:$PATH"
cd "$DESKTOP_DIR"
# Stop only this checkout's QA process, leaving the installed production app running.
BUNDLE="$DESKTOP_DIR/src-tauri/target/release/bundle/macos/DevClash QA.app"
while IFS= read -r process_id; do [[ -n "$process_id" ]] && kill "$process_id"; done < <(ps -axo pid=,command= | awk -v binary="$BUNDLE/Contents/MacOS/devclash-desktop" 'substr($0,index($0,$2)) == binary {print $1}')
# Offline by default. An explicit test endpoint still uses the isolated QA credential scope.
export DEVCLASH_API_URL="${DEVCLASH_API_URL-}"
export DEVCLASH_DATA_DIR="${DEVCLASH_DATA_DIR-$DESKTOP_DIR/artifacts/local-run/data}"
export DEVCLASH_QUOTAS_DISABLED="${DEVCLASH_QUOTAS_DISABLED-1}"
export DEVCLASH_CURSOR_FETCH_DISABLED="${DEVCLASH_CURSOR_FETCH_DISABLED-1}"
# Empty roots suppress automatic discovery; select synthetic folders via explicit overrides.
export DEVCLASH_CLAUDE_DIRS="${DEVCLASH_CLAUDE_DIRS-}"
export DEVCLASH_CODEX_DIRS="${DEVCLASH_CODEX_DIRS-}"
export DEVCLASH_CURSOR_DIRS="${DEVCLASH_CURSOR_DIRS-}"
export DEVCLASH_OPENCODE_DIRS="${DEVCLASH_OPENCODE_DIRS-}"
if "$VERIFY"; then npm test; cargo test --manifest-path src-tauri/Cargo.toml --lib; fi
# Local ad-hoc bundle, without generating an unsigned package for the update feed.
npm run tauri -- build --bundles app --config '{"productName":"DevClash QA","identifier":"ai.devclash.desktop.qa","bundle":{"createUpdaterArtifacts":false,"macOS":{"signingIdentity":"-"}}}'
/usr/bin/codesign --verify --deep --strict "$BUNDLE"
LOG_DIR="$DESKTOP_DIR/artifacts/local-run"
mkdir -p "$LOG_DIR"
OPEN_ARGS=(-n "$BUNDLE" --stdout "$LOG_DIR/stdout.log" --stderr "$LOG_DIR/stderr.log")
# Pass only app-specific overrides to LaunchServices. Never include provider tokens.
for name in DEVCLASH_API_URL DEVCLASH_APP_URL DEVCLASH_DATA_DIR DEVCLASH_QUOTAS_DISABLED DEVCLASH_CURSOR_FETCH_DISABLED DEVCLASH_CLAUDE_DIRS DEVCLASH_CODEX_DIRS DEVCLASH_CURSOR_DIRS DEVCLASH_OPENCODE_DIRS DEVCLASH_WINDOW_TRACE DEVCLASH_TRAY_TRACE; do
  if [[ "${!name+x}" == x ]]; then OPEN_ARGS+=(--env "$name=${!name}"); fi
done
/usr/bin/open "${OPEN_ARGS[@]}"
if "$VERIFY"; then
  for attempt in {1..30}; do if pgrep -f "$BUNDLE/Contents/MacOS/devclash-desktop$" >/dev/null; then printf 'DevClash local macOS bundle is running.\n'; exit 0; fi; sleep 0.2; done
  printf 'DevClash launch could not be verified; inspect artifacts/local-run/stderr.log.\n' >&2; exit 1
fi
