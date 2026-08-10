#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND_DIR="$REPOSITORY_DIR/frontend"
APP_URL="${AISSENCE_APP_URL:-http://127.0.0.1:5173}"
API_URL="${AISSENCE_API_URL:-http://127.0.0.1:8787}"
OPEN_BROWSER=false

if [[ "${1:-}" == "--open" ]]; then
  OPEN_BROWSER=true
elif [[ -n "${1:-}" ]]; then
  echo "Usage: scripts/dev.sh [--open]" >&2
  exit 2
fi

for command in cargo curl docker pnpm supabase; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 1
  fi
done

if ! docker info >/dev/null 2>&1; then
  echo "OrbStack or Docker is not running. Start it, then try again." >&2
  exit 1
fi

cd "$FRONTEND_DIR"
echo "Starting local Supabase (Mailpit is included)…"
if ! supabase_start_output="$(supabase start 2>&1)"; then
  printf '%s\n' "$supabase_start_output" >&2
  exit 1
fi

echo "Applying local database migrations…"
supabase migration up --local

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

read_first_status_value() {
  local requested value
  for requested in "$@"; do
    if value="$(read_status_value "$requested")"; then
      printf '%s' "$value"
      return 0
    fi
  done
  return 1
}

SUPABASE_URL="$(read_first_status_value API_URL)"
SUPABASE_PUBLISHABLE_KEY="$(read_first_status_value PUBLISHABLE_KEY ANON_KEY)"
SUPABASE_DATABASE_URL="$(read_first_status_value DB_URL)"
SUPABASE_JWT_SECRET_VALUE="$(read_first_status_value JWT_SECRET)"
MAILPIT_URL="$(read_first_status_value MAILPIT_URL INBUCKET_URL)"
STUDIO_URL="$(read_first_status_value STUDIO_URL)"

if [[ -z "$SUPABASE_URL" || -z "$SUPABASE_PUBLISHABLE_KEY" || -z "$SUPABASE_DATABASE_URL" || -z "$SUPABASE_JWT_SECRET_VALUE" ]]; then
  echo "Could not read the local Supabase connection settings." >&2
  exit 1
fi

export PUBLIC_SUPABASE_URL="$SUPABASE_URL"
export PUBLIC_SUPABASE_PUBLISHABLE_KEY="$SUPABASE_PUBLISHABLE_KEY"
export PUBLIC_SITE_URL="$APP_URL"
export AISSENCE_API_URL="$API_URL"
export AISSENCE_PUBLIC_URL="$API_URL"
export AISSENCE_SITE_URL="$APP_URL"
export DATABASE_URL="$SUPABASE_DATABASE_URL"
export SUPABASE_JWT_SECRET="$SUPABASE_JWT_SECRET_VALUE"

echo
echo "Aissence: $APP_URL"
echo "API:       $API_URL"
echo "Database:  local Supabase Postgres"
echo "Mailpit:   $MAILPIT_URL"
echo "Studio:    $STUDIO_URL"
echo
echo "CLI test:  cargo run -p aissence -- sync --server $API_URL /path/to/project"
echo "Press Ctrl-C to stop the frontend and API. Supabase will remain running."

SERVER_PID=""
FRONTEND_PID=""
cleanup() {
  if [[ -n "$FRONTEND_PID" ]]; then
    kill "$FRONTEND_PID" >/dev/null 2>&1 || true
    wait "$FRONTEND_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

if curl --silent --fail "$API_URL/health" >/dev/null 2>&1; then
  echo "Reusing the Aissence API already running at $API_URL."
else
  (
    cd "$REPOSITORY_DIR"
    exec cargo run -p aissence-server
  ) &
  SERVER_PID="$!"
  for _ in {1..120}; do
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

(
  cd "$FRONTEND_DIR"
  exec pnpm dev --host 127.0.0.1 --port 5173 --strictPort
) &
FRONTEND_PID="$!"
for _ in {1..120}; do
  if curl --silent --fail "$APP_URL" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$FRONTEND_PID" >/dev/null 2>&1; then
    wait "$FRONTEND_PID"
    exit 1
  fi
  sleep 0.25
done
if ! curl --silent --fail "$APP_URL" >/dev/null 2>&1; then
  echo "The Aissence frontend did not become ready." >&2
  exit 1
fi

if [[ "$OPEN_BROWSER" == true ]]; then
  if [[ -x /usr/bin/open ]]; then
    if ! /usr/bin/open "$APP_URL"; then
      echo "Could not open Aissence automatically: $APP_URL" >&2
    fi
    if ! /usr/bin/open "$MAILPIT_URL"; then
      echo "Could not open Mailpit automatically: $MAILPIT_URL" >&2
    fi
  else
    echo "Browser opening is only supported automatically on macOS." >&2
  fi
fi

wait "$FRONTEND_PID"
