#!/usr/bin/env bash
# Shared infrastructure functions for E2E tests.
# Source this file; do not execute directly.

set -euo pipefail

# ---- Defaults (override before sourcing or via env) ----
PROJECT_DIR="${PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
TESTS_DIR="${TESTS_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
POLL_INTERVAL="${POLL_INTERVAL:-2}"

NAMEROUTE_PID=""
TMPCONFIG=""
NAMEROUTE_LOG=""

# ---- Build ----
build_nameroute() {
    echo "=== Building nameroute ==="
    cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1 | tail -1
}

# ---- Config generation ----
generate_config() {
    TMPCONFIG=$(mktemp /tmp/nameroute-e2e-XXXXXX.toml)
    cat > "$TMPCONFIG" <<EOF
[general]
log_level = "info"
log_output = "stderr"
[docker]
poll_interval = $POLL_INTERVAL
[backend]
connect_timeout = 5
connect_retries = 3
[listeners.postgres]
protocol = "postgres"
bind = "127.0.0.1:$PG_PORT"
[listeners.mysql]
protocol = "mysql"
bind = "127.0.0.1:$MYSQL_PORT"
[listeners.smtp]
protocol = "smtp"
bind = "127.0.0.1:10025"
[smtp]
mailbox_dir = "/tmp/nameroute-e2e-mailbox"
EOF
}

# ---- Start / Stop ----
start_nameroute() {
    local bin="$PROJECT_DIR/target/debug/nameroute"
    NAMEROUTE_LOG=$(mktemp /tmp/nameroute-e2e-log-XXXXXX.txt)
    "$bin" --config "$TMPCONFIG" 2>"$NAMEROUTE_LOG" &
    NAMEROUTE_PID=$!
    sleep $((POLL_INTERVAL + 2))
    if ! kill -0 "$NAMEROUTE_PID" 2>/dev/null; then
        echo "ERROR: nameroute died on startup"
        cat "$NAMEROUTE_LOG" | tail -20
        exit 1
    fi
    echo "  nameroute running (pid=$NAMEROUTE_PID)"
}

stop_nameroute() {
    if [ -n "$NAMEROUTE_PID" ] && kill -0 "$NAMEROUTE_PID" 2>/dev/null; then
        kill "$NAMEROUTE_PID" 2>/dev/null || true
        wait "$NAMEROUTE_PID" 2>/dev/null || true
    fi
    NAMEROUTE_PID=""
}

# ---- Healthcheck helpers ----

# Wait for a docker compose service to become healthy.
# Usage: wait_healthy_compose <compose_project> <compose_file> <service> [timeout_secs]
wait_healthy_compose() {
    local project="$1" file="$2" svc="$3" timeout="${4:-60}"
    for i in $(seq 1 "$timeout"); do
        STATUS=$(docker compose -p "$project" -f "$file" \
            ps "$svc" --format json 2>/dev/null \
            | grep -o '"Health":"[^"]*"' | head -1 | sed 's/"Health":"//;s/"//' || true)
        if [ "$STATUS" = "healthy" ]; then
            echo "  $svc healthy (${i}s)"
            return 0
        fi
        sleep 1
    done
    echo "  ERROR: $svc did not become healthy within ${timeout}s"
    return 1
}

# Wait for a standalone docker container to become healthy.
# Usage: wait_healthy_container <container_name> [timeout_secs]
wait_healthy_container() {
    local name="$1" timeout="${2:-90}"
    for i in $(seq 1 "$timeout"); do
        STATUS=$(docker inspect --format='{{.State.Health.Status}}' "$name" 2>/dev/null || true)
        if [ "$STATUS" = "healthy" ]; then
            echo "  $name healthy (${i}s)"
            return 0
        fi
        sleep 1
    done
    echo "  ERROR: $name did not become healthy within ${timeout}s"
    return 1
}

# ---- Cleanup helpers ----
cleanup_tmpfiles() {
    [ -n "$TMPCONFIG" ] && rm -f "$TMPCONFIG"
    [ -n "$NAMEROUTE_LOG" ] && rm -f "$NAMEROUTE_LOG"
    TMPCONFIG=""
    NAMEROUTE_LOG=""
}
