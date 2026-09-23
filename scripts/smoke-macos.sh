#!/usr/bin/env bash
# macOS smoke check (T072).
#
# Not an end-to-end test, and deliberately not labelled one: it proves the application
# starts, paints and exits cleanly. It exercises no user journey. macOS has no WebDriver
# for its platform webview, so this is the coverage that platform can have — recorded in
# Appendix A, A-E2E and in the plan's Complexity Tracking.
set -euo pipefail

# Workspace root, not the member: F002 made this three crates and Cargo moved the artifacts.
BINARY="target/debug/apex-shell"
DATA_DIR="$(mktemp -d)"
SHOT="${1:-reports/macos/smoke.png}"

cleanup() { rm -rf "$DATA_DIR"; [[ -n "${APP_PID:-}" ]] && kill "$APP_PID" 2>/dev/null || true; }
trap cleanup EXIT

cargo build --manifest-path client/core/Cargo.toml
mkdir -p "$(dirname "$SHOT")"

APEX_DATA_DIR="$DATA_DIR" "$BINARY" &
APP_PID=$!

# The window is hidden until the readiness signal, so its appearance IS the signal.
for _ in $(seq 1 30); do
  if screencapture -x -o "$SHOT" 2>/dev/null && [[ -s "$SHOT" ]]; then break; fi
  sleep 1
done

if ! kill -0 "$APP_PID" 2>/dev/null; then
  echo "smoke: application exited before it became visible" >&2
  exit 1
fi

kill "$APP_PID"
wait "$APP_PID" 2>/dev/null || true

# An orphaned process after quit is a defect (FR-019).
if pgrep -x apex-shell >/dev/null 2>&1; then
  echo "smoke: apex-shell survived quit" >&2
  exit 1
fi

echo "smoke: started, painted, exited cleanly; screenshot at $SHOT"
