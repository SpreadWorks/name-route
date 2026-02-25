#!/usr/bin/env bash
set -euo pipefail

PASS=0
FAIL=0

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# Helper: wait until nameroute status succeeds on a given port (up to 10s)
wait_for_port() {
    local port="$1"
    for i in $(seq 1 20); do
        if nameroute -m "$port" status 2>&1 | grep -q "Daemon is running"; then
            return 0
        fi
        sleep 0.5
    done
    return 1
}

echo "=== nameroute deb package integration tests ==="
echo ""

# -------------------------------------------------------
# Test 1: Binary is installed at /usr/bin/nameroute
# -------------------------------------------------------
echo "[Test 1] Binary installed"
if [ -x /usr/bin/nameroute ]; then
    pass "binary exists at /usr/bin/nameroute"
else
    fail "binary not found at /usr/bin/nameroute"
fi

# -------------------------------------------------------
# Test 2: Config is installed at /etc/nameroute/config.toml
# -------------------------------------------------------
echo "[Test 2] Config installed"
if [ -f /etc/nameroute/config.toml ]; then
    pass "config exists at /etc/nameroute/config.toml"
else
    fail "config not found at /etc/nameroute/config.toml"
fi

# -------------------------------------------------------
# Test 3: Config contains management_port
# -------------------------------------------------------
echo "[Test 3] Config contains management_port"
if grep -q 'management_port' /etc/nameroute/config.toml; then
    pass "management_port found in config"
else
    fail "management_port not found in config"
fi

# -------------------------------------------------------
# Test 4: Daemon starts and listens on default port 14321
# -------------------------------------------------------
echo "[Test 4] Daemon starts on default port"
nameroute serve --config /etc/nameroute/config.toml &
DAEMON_PID=$!

if wait_for_port 14321; then
    pass "daemon is running and management port 14321 is reachable"
else
    fail "daemon failed to start or management port 14321 is NOT reachable"
fi

# -------------------------------------------------------
# Test 5: nameroute status works via TCP
# -------------------------------------------------------
echo "[Test 5] CLI status command"
if nameroute status 2>&1 | grep -q "Daemon is running"; then
    pass "status command works"
else
    fail "status command failed"
fi

# -------------------------------------------------------
# Test 6: nameroute list works via TCP
# -------------------------------------------------------
echo "[Test 6] CLI list command"
if nameroute list 2>&1 | grep -q "No routes registered"; then
    pass "list command works"
else
    fail "list command failed"
fi

# -------------------------------------------------------
# Test 7: nameroute add + list + remove
# -------------------------------------------------------
echo "[Test 7] add/list/remove cycle"
nameroute add http myapp 127.0.0.1:3000 2>&1
LIST_OUT=$(nameroute list 2>&1)
if echo "$LIST_OUT" | grep -q "myapp"; then
    pass "add + list works"
else
    fail "route not found after add"
fi

nameroute remove http myapp 2>&1
LIST_OUT2=$(nameroute list 2>&1)
if echo "$LIST_OUT2" | grep -q "No routes registered"; then
    pass "remove works"
else
    fail "route still present after remove"
fi

# -------------------------------------------------------
# Test 8: Duplicate daemon detection (port already in use)
# -------------------------------------------------------
echo "[Test 8] Duplicate daemon detection"
DUP_LOG=$(mktemp)
# Run second daemon in a subshell to capture its stderr
( nameroute serve --config /etc/nameroute/config.toml 2>&1 ) > "$DUP_LOG" &
DUP_PID=$!
sleep 3

# Check if the error message was logged
if grep -q "Management port already in use" "$DUP_LOG"; then
    pass "duplicate daemon detected port conflict"
else
    fail "no port conflict message from second daemon"
fi
kill "$DUP_PID" 2>/dev/null || true
wait "$DUP_PID" 2>/dev/null || true
rm -f "$DUP_LOG"

# Stop first daemon
kill "$DAEMON_PID" 2>/dev/null || true
wait "$DAEMON_PID" 2>/dev/null || true
sleep 1

# -------------------------------------------------------
# Test 9: --management-port flag
# -------------------------------------------------------
echo "[Test 9] --management-port flag"
nameroute -m 9999 serve --config /etc/nameroute/config.toml &
DAEMON2_PID=$!

if wait_for_port 9999; then
    pass "custom management port 9999 is reachable"
else
    fail "custom management port 9999 is NOT reachable"
fi

if nameroute -m 9999 status 2>&1 | grep -q "Daemon is running"; then
    pass "status via -m 9999 works"
else
    fail "status via -m 9999 failed"
fi

kill "$DAEMON2_PID" 2>/dev/null || true
wait "$DAEMON2_PID" 2>/dev/null || true
sleep 1

# -------------------------------------------------------
# Test 10: NAMEROUTE_PORT env var
# -------------------------------------------------------
echo "[Test 10] NAMEROUTE_PORT env var"
nameroute -m 7777 serve --config /etc/nameroute/config.toml &
DAEMON3_PID=$!

if wait_for_port 7777; then
    :
else
    fail "daemon on port 7777 did not start"
fi

if NAMEROUTE_PORT=7777 nameroute status 2>&1 | grep -q "Daemon is running"; then
    pass "status via NAMEROUTE_PORT=7777 works"
else
    fail "status via NAMEROUTE_PORT=7777 failed"
fi

kill "$DAEMON3_PID" 2>/dev/null || true
wait "$DAEMON3_PID" 2>/dev/null || true
sleep 1

# -------------------------------------------------------
# Test 11: No stale socket file after daemon stops
# -------------------------------------------------------
echo "[Test 11] No stale socket file"
if [ ! -e /tmp/nameroute.sock ]; then
    pass "no stale socket file at /tmp/nameroute.sock"
else
    fail "stale socket file found at /tmp/nameroute.sock"
fi

# -------------------------------------------------------
# Summary
# -------------------------------------------------------
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
