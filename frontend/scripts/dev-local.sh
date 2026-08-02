#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPOSITORY_DIR="$(cd "$PROJECT_DIR/.." && pwd)"
APP_URL="${AISSENCE_APP_URL:-http://127.0.0.1:5173}"
API_URL="${AISSENCE_API_URL:-http://127.0.0.1:8787}"
OPEN_BROWSER=false

if [[ "${1:-}" == "--open" ]]; then
  OPEN_BROWSER=true
elif [[ -n "${1:-}" ]]; then
  echo "Usage: pnpm dev:local [--open]" >&2
  exit 2
fi

for command in cargo curl docker supabase pnpm; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 1
  fi
done

if ! docker info >/dev/null 2>&1; then
  echo "OrbStack or Docker is not running. Start it, then try again." >&2
  exit 1
fi

cd "$PROJECT_DIR"
echo "Starting local Supabase (Mailpit is included)…"
if ! supabase_start_output="$(supabase start 2>&1)"; then
  printf '%s\n' "$supabase_start_output" >&2
  exit 1
fi

STATUS_ENV="$(supabase status -o env)"

read_status_value() {
  local requested="$1"
  local line value
  while IFS= read -r line; do
    if [[ "$line" == "$requested="* ]]; then
      value="${line#*=}"
      value="${value#\"}"
      value="${value%\"}"
      printf '%s' "$value"
      return 0
    fi
  done <<< "$STATUS_ENV"
  return 1
}

SUPABASE_URL="$(read_status_value API_URL)"
SUPABASE_PUBLISHABLE_KEY="$(read_status_value PUBLISHABLE_KEY)"
MAILPIT_URL="$(read_status_value MAILPIT_URL)"
STUDIO_URL="$(read_status_value STUDIO_URL)"

if [[ -z "$SUPABASE_URL" || -z "$SUPABASE_PUBLISHABLE_KEY" ]]; then
  echo "Could not read the local Supabase URL or publishable key." >&2
  exit 1
fi

echo
echo "Aissence: $APP_URL"
echo "API:       $API_URL"
echo "Mailpit:   $MAILPIT_URL"
echo "Studio:    $STUDIO_URL"
echo
echo "Press Ctrl-C to stop Aissence. Supabase will remain running."

export PUBLIC_SUPABASE_URL="$SUPABASE_URL"
export PUBLIC_SUPABASE_PUBLISHABLE_KEY="$SUPABASE_PUBLISHABLE_KEY"
export PUBLIC_SITE_URL="$APP_URL"
export AISSENCE_API_URL="$API_URL"
export SUPABASE_ANON_KEY="$SUPABASE_PUBLISHABLE_KEY"

SERVER_PID=""
if ! curl --silent --fail "$API_URL/health" >/dev/null 2>&1; then
  if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
    echo "OPENROUTER_API_KEY is required to start the hosted summary worker." >&2
    exit 1
  fi
  (
    cd "$REPOSITORY_DIR"
    exec cargo run -p aissence-server
  ) &
  SERVER_PID="$!"
  for _ in {1..60}; do
    if curl --silent --fail "$API_URL/health" >/dev/null 2>&1; then
      break
    fi
    if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
      wait "$SERVER_PID"
      exit 1
    fi
    sleep 0.5
  done
  if ! curl --silent --fail "$API_URL/health" >/dev/null 2>&1; then
    echo "The Aissence API did not become ready." >&2
    exit 1
  fi
fi

cleanup() {
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

if [[ "$OPEN_BROWSER" == true ]]; then
  if command -v open >/dev/null 2>&1; then
    open "$APP_URL" "$MAILPIT_URL"
  else
    echo "Browser opening is only supported automatically on macOS." >&2
  fi
fi

pnpm dev --host 127.0.0.1 --port 5173
