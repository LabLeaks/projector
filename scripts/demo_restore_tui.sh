#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MISE_CACHE_DIR="${MISE_CACHE_DIR:-/tmp/projector-mise-cache}"
PORT="${PORT:-$((20000 + RANDOM % 20000))}"
DEMO_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/projector-restore-demo.XXXXXX")"
REPO_DIR="$DEMO_ROOT/repo"
STATE_DIR="$DEMO_ROOT/server-state"
SERVER_LOG="$DEMO_ROOT/server.log"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

mkdir -p "$REPO_DIR" "$STATE_DIR"
cd "$REPO_DIR"
git init -q
mkdir -p .jj
cat > .gitignore <<'EOF'
private/
notes/
EOF

(
  cd "$ROOT_DIR"
  MISE_CACHE_DIR="$MISE_CACHE_DIR" mise exec -- cargo run -p projector-server --bin projector-server -- \
    serve --addr "127.0.0.1:$PORT" --state-dir "$STATE_DIR"
) >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
sleep 1

run_projector() {
  (
    cd "$REPO_DIR"
    MISE_CACHE_DIR="$MISE_CACHE_DIR" mise exec -- cargo run --manifest-path "$ROOT_DIR/Cargo.toml" -p projector-cli --bin projector -- "$@"
  )
}

run_projector sync --server "127.0.0.1:$PORT" private notes >/dev/null

mkdir -p private/briefs
cat > private/briefs/restore-demo.html <<'EOF'
<p>revision one</p>
EOF
run_projector sync >/dev/null

cat > private/briefs/restore-demo.html <<'EOF'
<p>revision two</p>
EOF
run_projector sync >/dev/null

cat > private/briefs/restore-demo.html <<'EOF'
<p>revision three</p>
EOF
run_projector sync >/dev/null

clear
cat <<EOF
projector restore TUI demo

repo:   $REPO_DIR
server: 127.0.0.1:$PORT
file:   private/briefs/restore-demo.html

keys:
  up/down or j/k  move between revisions
  pageup/pagedown scroll diff
  enter           preview selected restore
  q               cancel

re-run with:
  cd "$REPO_DIR"
  MISE_CACHE_DIR="$MISE_CACHE_DIR" mise exec -- cargo run -p projector-cli --bin projector -- restore --confirm private/briefs/restore-demo.html

Launching browser...
EOF

run_projector restore private/briefs/restore-demo.html
