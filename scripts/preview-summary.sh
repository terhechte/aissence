#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_URL="${AISSENCE_APP_URL:-http://127.0.0.1:5173}"
DEV_LOG="${AISSENCE_DEV_LOG:-/tmp/aissence-dev.log}"
ALL_PROMPTS=false
FALLBACK_MODEL=""
POSITIONAL=()

preview_server_ready() {
  curl --silent --fail --max-time 2 "$APP_URL/api/dev/previews" >/dev/null 2>&1
}

usage() {
  cat <<'EOF'
Usage: scripts/preview-summary.sh [--all-prompts] [--fallback-model <model>] <repository-path> <openrouter-model>

Deletes <repository-path>/.aissence, regenerates it with local OpenRouter
summaries, starts the development stack when needed, uploads the result to a
loopback-only preview, and opens that preview in the default browser.

Environment:
  OPENROUTER_API_KEY  OpenRouter credential used by the CLI
  OPENROUTER_FALLBACK_MODEL
                      Optional fallback model; overridden by --fallback-model
  AISSENCE_APP_URL    Frontend URL (default: http://127.0.0.1:5173)
  AISSENCE_DEV_LOG    Background dev-stack log (default: /tmp/aissence-dev.log)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all-prompts)
      ALL_PROMPTS=true
      shift
      ;;
    --fallback-model)
      if [[ $# -lt 2 || -z "$2" || "$2" == -* ]]; then
        echo "--fallback-model requires a model identifier" >&2
        usage >&2
        exit 2
      fi
      FALLBACK_MODEL="$2"
      shift 2
      ;;
    --fallback-model=*)
      FALLBACK_MODEL="${1#*=}"
      if [[ -z "$FALLBACK_MODEL" ]]; then
        echo "--fallback-model requires a model identifier" >&2
        usage >&2
        exit 2
      fi
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      POSITIONAL+=("$@")
      break
      ;;
    -*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

if [[ ${#POSITIONAL[@]} -ne 2 ]]; then
  usage >&2
  exit 2
fi

TARGET_INPUT="${POSITIONAL[0]}"
MODEL="${POSITIONAL[1]}"
if [[ ! -d "$TARGET_INPUT" ]]; then
  echo "Repository folder does not exist: $TARGET_INPUT" >&2
  exit 1
fi
TARGET_DIR="$(cd "$TARGET_INPUT" && pwd -P)"
if [[ ! -e "$TARGET_DIR/.git" ]]; then
  echo "Not a Git worktree: $TARGET_DIR" >&2
  exit 1
fi

for command in cargo curl; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 1
  fi
done

SUMMARY_ARGS=(summarize --mode local --api openrouter --model "$MODEL")
if [[ -n "$FALLBACK_MODEL" ]]; then
  SUMMARY_ARGS+=(--fallback-model "$FALLBACK_MODEL")
fi
if [[ "$ALL_PROMPTS" == true ]]; then
  SUMMARY_ARGS+=(--all-prompts)
fi
SUMMARY_ARGS+=("$TARGET_DIR")

echo "Removing the previous summary cache: $TARGET_DIR/.aissence"
rm -f -- "$TARGET_DIR/.aissence"

echo "Generating summaries with OpenRouter model: $MODEL"
if [[ -n "$FALLBACK_MODEL" ]]; then
  echo "Using fallback OpenRouter model: $FALLBACK_MODEL"
elif [[ -n "${OPENROUTER_FALLBACK_MODEL:-}" ]]; then
  echo "Using fallback OpenRouter model from OPENROUTER_FALLBACK_MODEL: $OPENROUTER_FALLBACK_MODEL"
fi
(
  cd "$REPOSITORY_DIR"
  cargo run -p aissence -- "${SUMMARY_ARGS[@]}"
)

AISSENCE_FILE="$TARGET_DIR/.aissence"
if [[ ! -f "$AISSENCE_FILE" ]]; then
  echo "The CLI completed without creating $AISSENCE_FILE" >&2
  exit 1
fi

if ! preview_server_ready; then
  echo "Starting the development stack in the background (log: $DEV_LOG)"
  nohup "$REPOSITORY_DIR/scripts/dev.sh" >"$DEV_LOG" 2>&1 </dev/null &
  DEV_PID=$!
  READY=false
  for _ in {1..240}; do
    if preview_server_ready; then
      READY=true
      break
    fi
    if ! kill -0 "$DEV_PID" >/dev/null 2>&1; then
      echo "The development stack stopped before becoming ready. Last log lines:" >&2
      tail -n 40 "$DEV_LOG" >&2 || true
      exit 1
    fi
    sleep 0.5
  done
  if [[ "$READY" != true ]]; then
    echo "The frontend did not become ready at $APP_URL. Last log lines:" >&2
    tail -n 40 "$DEV_LOG" >&2 || true
    exit 1
  fi
else
  echo "Reusing the development server at $APP_URL"
fi

echo "Uploading the generated file to the local preview"
PREVIEW_URL="$(curl --silent --show-error --fail \
  --request POST \
  --header 'Content-Type: application/vnd.aissence+json' \
  --data-binary "@$AISSENCE_FILE" \
  "$APP_URL/api/dev/previews")"
PREVIEW_URL="${PREVIEW_URL%$'\n'}"

echo "Opening $PREVIEW_URL"
if command -v open >/dev/null 2>&1; then
  open "$PREVIEW_URL"
elif command -v xdg-open >/dev/null 2>&1; then
  xdg-open "$PREVIEW_URL"
else
  echo "No browser opener was found; open the URL above manually."
fi
