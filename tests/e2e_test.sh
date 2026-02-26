#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
NAMEROUTE_PID=""
ECHO_HTTP_PID=""
TMPCONFIG=""
MAILBOX_DIR=""
NAMEROUTE_LOG=""
COMPOSE_PROJECT="nameroute-e2e-$$"

cleanup() {
    echo "--- cleanup ---"
    if [ -n "$ECHO_HTTP_PID" ] && kill -0 "$ECHO_HTTP_PID" 2>/dev/null; then
        kill "$ECHO_HTTP_PID" 2>/dev/null || true
        wait "$ECHO_HTTP_PID" 2>/dev/null || true
    fi
    if [ -n "$NAMEROUTE_PID" ] && kill -0 "$NAMEROUTE_PID" 2>/dev/null; then
        kill "$NAMEROUTE_PID" 2>/dev/null || true
        wait "$NAMEROUTE_PID" 2>/dev/null || true
    fi
    docker compose -p "$COMPOSE_PROJECT" -f "$SCRIPT_DIR/docker-compose.yml" down -v 2>/dev/null || true
    [ -n "$TMPCONFIG" ] && rm -f "$TMPCONFIG"
    [ -n "$MAILBOX_DIR" ] && rm -rf "$MAILBOX_DIR"
    [ -n "$NAMEROUTE_LOG" ] && rm -f "$NAMEROUTE_LOG"
}
trap cleanup EXIT

PASS=0
FAIL=0

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# Find an available port by binding to :0 and reading the assigned port
find_free_port() {
    python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

# ---- 1. Build nameroute ----
if [ -n "${NAMEROUTE_BIN:-}" ]; then
    echo "=== Using pre-built nameroute: $NAMEROUTE_BIN ==="
else
    echo "=== Building nameroute ==="
    cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1
    NAMEROUTE_BIN="$PROJECT_DIR/target/debug/nameroute"
fi

# ---- 2. Start containers ----
echo "=== Starting containers ==="
docker compose -p "$COMPOSE_PROJECT" -f "$SCRIPT_DIR/docker-compose.yml" up -d

# ---- 3. Wait for all containers to be healthy ----
echo "=== Waiting for all containers healthcheck ==="
SERVICES="postgres_multi postgres_second mysql_multi mysql_second httpd_web"
for svc in $SERVICES; do
    echo "  Waiting for $svc ..."
    for i in $(seq 1 60); do
        STATUS=$(docker compose -p "$COMPOSE_PROJECT" -f "$SCRIPT_DIR/docker-compose.yml" \
            ps "$svc" --format json 2>/dev/null \
            | grep -o '"Health":"[^"]*"' | head -1 | sed 's/"Health":"//;s/"//' || true)
        if [ "$STATUS" = "healthy" ]; then
            echo "  $svc is healthy (attempt $i)"
            break
        fi
        if [ "$i" -eq 60 ]; then
            echo "  ERROR: $svc did not become healthy in time"
            docker compose -p "$COMPOSE_PROJECT" -f "$SCRIPT_DIR/docker-compose.yml" logs "$svc"
            exit 1
        fi
        sleep 1
    done
done

# ---- 4. Allocate free ports ----
echo "=== Allocating free ports ==="
PG_PORT=$(find_free_port)
MYSQL_PORT=$(find_free_port)
SMTP_PORT=$(find_free_port)
HTTP_PORT=$(find_free_port)
ECHO_HTTP_PORT=$(find_free_port)
echo "  PG=$PG_PORT MYSQL=$MYSQL_PORT SMTP=$SMTP_PORT HTTP=$HTTP_PORT ECHO_HTTP=$ECHO_HTTP_PORT"

# ---- 5. Start echo-http-server for static route testing ----
ECHO_HTTP_BIN="${ECHO_HTTP_BIN:-$PROJECT_DIR/target/debug/examples/echo_http_server}"
echo "=== Starting echo-http-server on port $ECHO_HTTP_PORT ==="
"$ECHO_HTTP_BIN" --port "$ECHO_HTTP_PORT" --body "static-route-ok" &
ECHO_HTTP_PID=$!
sleep 0.5

if ! kill -0 "$ECHO_HTTP_PID" 2>/dev/null; then
    echo "ERROR: echo-http-server exited immediately"
    exit 1
fi

# ---- 6. Generate temporary config ----
POLL_INTERVAL=2

MAILBOX_DIR=$(mktemp -d /tmp/nameroute-e2e-mailbox-XXXXXX)
TMPCONFIG=$(mktemp /tmp/nameroute-e2e-XXXXXX.toml)
cat > "$TMPCONFIG" <<EOF
[general]
log_level = "debug"
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

[listeners.http]
protocol = "http"
bind = "127.0.0.1:$HTTP_PORT"

[listeners.smtp]
protocol = "smtp"
bind = "127.0.0.1:$SMTP_PORT"

[http]
base_domain = "localhost"

[smtp]
mailbox_dir = "$MAILBOX_DIR"

[[routes]]
protocol = "http"
key = "static1"
backend = "127.0.0.1:$ECHO_HTTP_PORT"
EOF

# ---- 7. Start nameroute ----
echo "=== Starting nameroute ==="
NAMEROUTE_LOG=$(mktemp /tmp/nameroute-e2e-log-XXXXXX.txt)
"$NAMEROUTE_BIN" --config "$TMPCONFIG" 2>"$NAMEROUTE_LOG" &
NAMEROUTE_PID=$!
sleep 1

if ! kill -0 "$NAMEROUTE_PID" 2>/dev/null; then
    echo "ERROR: nameroute exited immediately"
    cat "$NAMEROUTE_LOG"
    exit 1
fi

# ---- 8. Wait for Docker polling ----
echo "=== Waiting for Docker poll (${POLL_INTERVAL}s + 1s) ==="
sleep $((POLL_INTERVAL + 1))

# ======================================================================
#  PostgreSQL tests
# ======================================================================
echo ""
echo "=== PostgreSQL tests ==="

HAS_PSQL=false
command -v psql &>/dev/null && HAS_PSQL=true

if $HAS_PSQL; then
    export PGPASSWORD=pass

    # Container 1 / key=app
    echo "-- PG: Container 1 / key=app: SELECT --"
    RESULT=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d app -tAc "SELECT 'hello_app'" 2>/dev/null) || true
    if [ "$RESULT" = "hello_app" ]; then
        pass "PG container1 key=app: SELECT"
    else
        fail "PG container1 key=app: got '$RESULT'"
    fi

    echo "-- PG: Container 1 / key=app: DDL/DML --"
    DDL1=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d app -tAc "
        DROP TABLE IF EXISTS e2e_app;
        CREATE TABLE e2e_app (id serial PRIMARY KEY, val text);
        INSERT INTO e2e_app (val) VALUES ('from_app');
        SELECT count(*) FROM e2e_app;
    " 2>/dev/null) || true
    if [ "$DDL1" = "1" ]; then
        pass "PG container1 key=app: DDL/DML count=1"
    else
        fail "PG container1 key=app: DDL/DML got '$DDL1'"
    fi

    # Container 1 / key=app_extra (multi-key)
    echo "-- PG: Container 1 / key=app_extra (multi-key): SELECT --"
    RESULT2=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d app_extra -tAc "SELECT 'hello_extra'" 2>/dev/null) || true
    if [ "$RESULT2" = "hello_extra" ]; then
        pass "PG container1 key=app_extra: SELECT"
    else
        fail "PG container1 key=app_extra: got '$RESULT2'"
    fi

    # Multi-key isolation
    echo "-- PG: Multi-key isolation --"
    ISO=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d app -tAc "
        SELECT count(*) FROM information_schema.tables WHERE table_name = 'e2e_extra';
    " 2>/dev/null) || true
    if [ "$ISO" = "0" ]; then
        pass "PG multi-key isolation: e2e_extra not in app"
    else
        fail "PG multi-key isolation: got '$ISO'"
    fi

    # Container 2 / key=warehouse
    echo "-- PG: Container 2 / key=warehouse: SELECT --"
    RESULT3=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d warehouse -tAc "SELECT 'hello_warehouse'" 2>/dev/null) || true
    if [ "$RESULT3" = "hello_warehouse" ]; then
        pass "PG container2 key=warehouse: SELECT"
    else
        fail "PG container2 key=warehouse: got '$RESULT3'"
    fi

    echo "-- PG: Container 2 / key=warehouse: DDL/DML --"
    DDL2=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d warehouse -tAc "
        DROP TABLE IF EXISTS e2e_wh;
        CREATE TABLE e2e_wh (id serial PRIMARY KEY, val text);
        INSERT INTO e2e_wh (val) VALUES ('wh1'), ('wh2');
        SELECT count(*) FROM e2e_wh;
    " 2>/dev/null) || true
    if [ "$DDL2" = "2" ]; then
        pass "PG container2 key=warehouse: DDL/DML count=2"
    else
        fail "PG container2 key=warehouse: DDL/DML got '$DDL2'"
    fi

    # Cross-container isolation
    echo "-- PG: Cross-container isolation --"
    CROSS=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d warehouse -tAc "
        SELECT count(*) FROM information_schema.tables WHERE table_name = 'e2e_app';
    " 2>/dev/null) || true
    if [ "$CROSS" = "0" ]; then
        pass "PG cross-container isolation"
    else
        fail "PG cross-container isolation: got '$CROSS'"
    fi

    # Unknown database
    echo "-- PG: Unknown database --"
    ERR=$(psql -h 127.0.0.1 -p "$PG_PORT" -U user -d no_such_db -tAc "SELECT 1" 2>&1) || true
    if echo "$ERR" | grep -qi "error\|fatal"; then
        pass "PG unknown database: error returned"
    else
        fail "PG unknown database: got '$ERR'"
    fi

    unset PGPASSWORD
else
    echo "  psql not found; falling back to TCP check"
    if (echo -n "" | timeout 3 bash -c "cat > /dev/tcp/127.0.0.1/$PG_PORT" 2>/dev/null); then
        pass "PG TCP connection succeeded"
    else
        fail "PG TCP connection failed"
    fi
fi

# ======================================================================
#  MySQL tests
# ======================================================================
echo ""
echo "=== MySQL tests ==="

HAS_MYSQL=false
command -v mysql &>/dev/null && HAS_MYSQL=true

if $HAS_MYSQL; then
    MYSQL_CMD="mysql -h 127.0.0.1 -P $MYSQL_PORT -u root --ssl-mode=DISABLED"

    # Container 1 / key=myapp
    echo "-- MySQL: Container 1 / key=myapp: SELECT --"
    RESULT=$($MYSQL_CMD myapp -NBe "SELECT 'hello_myapp'" 2>&1) || true
    if [ "$RESULT" = "hello_myapp" ]; then
        pass "MySQL container1 key=myapp: SELECT"
    else
        fail "MySQL container1 key=myapp: got '$RESULT'"
    fi

    echo "-- MySQL: Container 1 / key=myapp: DDL/DML --"
    DDL=$($MYSQL_CMD myapp -NBe "
        DROP TABLE IF EXISTS e2e_myapp;
        CREATE TABLE e2e_myapp (id INT AUTO_INCREMENT PRIMARY KEY, val VARCHAR(255));
        INSERT INTO e2e_myapp (val) VALUES ('from_myapp');
        SELECT count(*) FROM e2e_myapp;
    " 2>&1) || true
    if [ "$DDL" = "1" ]; then
        pass "MySQL container1 key=myapp: DDL/DML count=1"
    else
        fail "MySQL container1 key=myapp: DDL/DML got '$DDL'"
    fi

    # Container 1 / key=myapp_alias (multi-key)
    echo "-- MySQL: Container 1 / key=myapp_alias (multi-key): SELECT --"
    RESULT2=$($MYSQL_CMD myapp_alias -NBe "SELECT 'hello_alias'" 2>&1) || true
    if [ "$RESULT2" = "hello_alias" ]; then
        pass "MySQL container1 key=myapp_alias: SELECT"
    else
        fail "MySQL container1 key=myapp_alias: got '$RESULT2'"
    fi

    echo "-- MySQL: Container 1 / key=myapp_alias: DDL/DML --"
    DDL_ALIAS=$($MYSQL_CMD myapp_alias -NBe "
        DROP TABLE IF EXISTS e2e_alias;
        CREATE TABLE e2e_alias (id INT AUTO_INCREMENT PRIMARY KEY, val VARCHAR(255));
        INSERT INTO e2e_alias (val) VALUES ('from_alias');
        SELECT count(*) FROM e2e_alias;
    " 2>&1) || true
    if [ "$DDL_ALIAS" = "1" ]; then
        pass "MySQL container1 key=myapp_alias: DDL/DML count=1"
    else
        fail "MySQL container1 key=myapp_alias: DDL/DML got '$DDL_ALIAS'"
    fi

    # Multi-key isolation: myapp should not see e2e_alias table
    echo "-- MySQL: Multi-key isolation --"
    ISO=$($MYSQL_CMD myapp -NBe "
        SELECT count(*) FROM information_schema.tables
        WHERE table_schema = 'myapp' AND table_name = 'e2e_alias';
    " 2>&1) || true
    if [ "$ISO" = "0" ]; then
        pass "MySQL multi-key isolation: e2e_alias not in myapp"
    else
        fail "MySQL multi-key isolation: got '$ISO'"
    fi

    # Container 2 / key=analytics
    echo "-- MySQL: Container 2 / key=analytics: SELECT --"
    RESULT3=$($MYSQL_CMD analytics -NBe "SELECT 'hello_analytics'" 2>&1) || true
    if [ "$RESULT3" = "hello_analytics" ]; then
        pass "MySQL container2 key=analytics: SELECT"
    else
        fail "MySQL container2 key=analytics: got '$RESULT3'"
    fi

    echo "-- MySQL: Container 2 / key=analytics: DDL/DML --"
    DDL2=$($MYSQL_CMD analytics -NBe "
        DROP TABLE IF EXISTS e2e_analytics;
        CREATE TABLE e2e_analytics (id INT AUTO_INCREMENT PRIMARY KEY, val VARCHAR(255));
        INSERT INTO e2e_analytics (val) VALUES ('row1'), ('row2');
        SELECT count(*) FROM e2e_analytics;
    " 2>&1) || true
    if [ "$DDL2" = "2" ]; then
        pass "MySQL container2 key=analytics: DDL/DML count=2"
    else
        fail "MySQL container2 key=analytics: DDL/DML got '$DDL2'"
    fi

    # Cross-container isolation
    echo "-- MySQL: Cross-container isolation --"
    CROSS=$($MYSQL_CMD analytics -NBe "
        SELECT count(*) FROM information_schema.tables
        WHERE table_schema = 'analytics' AND table_name = 'e2e_myapp';
    " 2>&1) || true
    if [ "$CROSS" = "0" ]; then
        pass "MySQL cross-container isolation"
    else
        fail "MySQL cross-container isolation: got '$CROSS'"
    fi

    # Unknown database
    echo "-- MySQL: Unknown database --"
    ERR=$($MYSQL_CMD no_such_db -NBe "SELECT 1" 2>&1) || true
    if echo "$ERR" | grep -qi "error\|unknown"; then
        pass "MySQL unknown database: error returned"
    else
        fail "MySQL unknown database: got '$ERR'"
    fi
else
    echo "  mysql CLI not found; falling back to TCP check"
    if (echo -n "" | timeout 3 bash -c "cat > /dev/tcp/127.0.0.1/$MYSQL_PORT" 2>/dev/null); then
        pass "MySQL TCP connection succeeded"
    else
        fail "MySQL TCP connection failed"
    fi
fi

# ======================================================================
#  SMTP tests (nameroute handles SMTP directly, no backend container)
# ======================================================================
echo ""
echo "=== SMTP tests ==="

# Helper: run an SMTP dialog via a separate bash process (avoids set -e issues in subshells)
smtp_send() {
    local RCPT_TO="$1"
    local BODY="$2"
    bash --norc -c '
        exec 3<>/dev/tcp/127.0.0.1/'"$SMTP_PORT"'

        read -t 5 -r LINE <&3; echo "$LINE"

        printf "EHLO test\r\n" >&3
        while read -t 5 -r LINE <&3; do
            echo "$LINE"
            case "$LINE" in 250\ *) break ;; esac
        done

        printf "MAIL FROM:<sender@test.local>\r\n" >&3
        read -t 5 -r LINE <&3; echo "$LINE"

        printf "RCPT TO:<'"$RCPT_TO"'>\r\n" >&3
        read -t 5 -r LINE <&3; echo "$LINE"

        printf "DATA\r\n" >&3
        read -t 5 -r LINE <&3; echo "$LINE"

        printf "'"$BODY"'\r\n.\r\n" >&3
        read -t 5 -r LINE <&3; echo "$LINE"

        printf "QUIT\r\n" >&3
        read -t 5 -r LINE <&3; echo "$LINE"

        exec 3>&-
    ' 2>/dev/null || true
}

# Test 1: Send email and verify response
echo "-- SMTP: Send email to user@example.com --"
SMTP_OUT=$(smtp_send "user@example.com" "Subject: Test1\r\n\r\nHello World")
if echo "$SMTP_OUT" | grep -q "^250 OK"; then
    pass "SMTP send: got 250 OK"
else
    fail "SMTP send: output was '$(echo "$SMTP_OUT" | tr '\n' '|')'"
fi

# Test 2: Verify email file was saved
echo "-- SMTP: Verify email file saved --"
sleep 0.5
EML_COUNT=$(find "$MAILBOX_DIR/example.com" -name "*.eml" 2>/dev/null | wc -l || echo 0)
if [ "$EML_COUNT" -ge 1 ]; then
    pass "SMTP mailbox: $EML_COUNT .eml file(s) in example.com/"
else
    fail "SMTP mailbox: no .eml found in $MAILBOX_DIR/example.com/"
fi

# Test 3: Send to a different domain
echo "-- SMTP: Send email to admin@other.org --"
SMTP_OUT2=$(smtp_send "admin@other.org" "Subject: Test2\r\n\r\nSecond email")
if echo "$SMTP_OUT2" | grep -q "^250 OK"; then
    pass "SMTP send to other.org: got 250 OK"
else
    fail "SMTP send to other.org: output was '$(echo "$SMTP_OUT2" | tr '\n' '|')'"
fi

# Test 4: Verify second domain directory
echo "-- SMTP: Verify other.org mailbox --"
sleep 0.5
EML_COUNT2=$(find "$MAILBOX_DIR/other.org" -name "*.eml" 2>/dev/null | wc -l || echo 0)
if [ "$EML_COUNT2" -ge 1 ]; then
    pass "SMTP mailbox: $EML_COUNT2 .eml file(s) in other.org/"
else
    fail "SMTP mailbox: no .eml found in $MAILBOX_DIR/other.org/"
fi

# Test 5: QUIT without sending email
echo "-- SMTP: QUIT only --"
QUIT_OUT=$(bash --norc -c '
    exec 3<>/dev/tcp/127.0.0.1/'"$SMTP_PORT"'
    read -t 5 -r LINE <&3
    printf "EHLO test\r\n" >&3
    while read -t 5 -r LINE <&3; do
        case "$LINE" in 250\ *) break ;; esac
    done
    printf "QUIT\r\n" >&3
    read -t 5 -r LINE <&3
    echo "$LINE"
    exec 3>&-
' 2>/dev/null || true)
if echo "$QUIT_OUT" | grep -q "^221"; then
    pass "SMTP QUIT: got 221"
else
    fail "SMTP QUIT: got '$QUIT_OUT'"
fi

# ======================================================================
#  HTTP tests
# ======================================================================
echo ""
echo "=== HTTP tests ==="

HAS_CURL=false
command -v curl &>/dev/null && HAS_CURL=true

if $HAS_CURL; then
    # Test 1: Docker Apache via Docker label discovery (key=web)
    echo "-- HTTP: Docker Apache key=web --"
    RESULT=$(curl -s -o /dev/null -w "%{http_code}" -H "Host: web.localhost" "http://127.0.0.1:$HTTP_PORT/" 2>/dev/null) || true
    if [ "$RESULT" = "200" ]; then
        pass "HTTP Docker Apache key=web: status 200"
    else
        fail "HTTP Docker Apache key=web: status '$RESULT'"
    fi

    # Test 2: Verify Apache response contains expected content
    echo "-- HTTP: Docker Apache response body --"
    BODY=$(curl -s -H "Host: web.localhost" "http://127.0.0.1:$HTTP_PORT/" 2>/dev/null) || true
    if echo "$BODY" | grep -qi "it works\|apache\|html"; then
        pass "HTTP Docker Apache: body contains expected content"
    else
        fail "HTTP Docker Apache: body was '$(echo "$BODY" | head -1)'"
    fi

    # Test 3: Static route via echo-http-server (key=static1)
    echo "-- HTTP: Static route key=static1 --"
    RESULT=$(curl -s -H "Host: static1.localhost" "http://127.0.0.1:$HTTP_PORT/" 2>/dev/null) || true
    if [ "$RESULT" = "static-route-ok" ]; then
        pass "HTTP static route key=static1: body correct"
    else
        fail "HTTP static route key=static1: got '$RESULT'"
    fi

    # Test 4: Unknown key returns 502
    echo "-- HTTP: Unknown key --"
    STATUS=$(curl -s -o /dev/null -w "%{http_code}" -H "Host: nosuchkey.localhost" "http://127.0.0.1:$HTTP_PORT/" 2>/dev/null) || true
    if [ "$STATUS" = "502" ]; then
        pass "HTTP unknown key: status 502"
    else
        fail "HTTP unknown key: status '$STATUS'"
    fi

    # Test 5: No subdomain returns 404
    echo "-- HTTP: No subdomain --"
    STATUS=$(curl -s -o /dev/null -w "%{http_code}" -H "Host: localhost" "http://127.0.0.1:$HTTP_PORT/" 2>/dev/null) || true
    if [ "$STATUS" = "404" ]; then
        pass "HTTP no subdomain: status 404"
    else
        fail "HTTP no subdomain: status '$STATUS'"
    fi
else
    echo "  curl not found; skipping HTTP tests"
fi

# ======================================================================
#  nameroute liveness check
# ======================================================================
echo ""
echo "=== Liveness check ==="
if kill -0 "$NAMEROUTE_PID" 2>/dev/null; then
    pass "nameroute still running after all tests"
else
    fail "nameroute crashed during tests"
fi

# ======================================================================
#  Summary
# ======================================================================
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
