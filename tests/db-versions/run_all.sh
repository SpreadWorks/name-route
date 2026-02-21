#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

source "$TESTS_DIR/lib/infra.sh"
source "$TESTS_DIR/lib/results.sh"

# Container names (prefixed to avoid collisions)
PREFIX="nr-dbver-$$"
PG14="${PREFIX}-pg14"
PG15="${PREFIX}-pg15"
PG16="${PREFIX}-pg16"
PG17="${PREFIX}-pg17"
MY57="${PREFIX}-mysql57"
MY80="${PREFIX}-mysql80"
MY84="${PREFIX}-mysql84"

cleanup() {
    echo ""
    echo "--- cleanup (db-versions) ---"
    stop_nameroute
    for c in "$PG14" "$PG15" "$PG16" "$PG17" "$MY57" "$MY80" "$MY84"; do
        docker rm -f "$c" 2>/dev/null || true
    done
    cleanup_tmpfiles
    cleanup_results_dir
}
trap cleanup EXIT

# ---- Build ----
build_nameroute

# ---- Start DB containers ----
echo "=== Starting DB version containers ==="

docker run -d --name "$PG14" -p 0:5432 \
    -e POSTGRES_USER=user -e POSTGRES_PASSWORD=pass -e POSTGRES_DB=app_pg14 \
    --label "name-route=[{\"protocol\":\"postgres\",\"key\":\"app_pg14\"}]" \
    --health-cmd "pg_isready -U user -d app_pg14" --health-interval 2s --health-timeout 3s --health-retries 15 \
    postgres:14 >/dev/null

docker run -d --name "$PG15" -p 0:5432 \
    -e POSTGRES_USER=user -e POSTGRES_PASSWORD=pass -e POSTGRES_DB=app_pg15 \
    --label "name-route=[{\"protocol\":\"postgres\",\"key\":\"app_pg15\"}]" \
    --health-cmd "pg_isready -U user -d app_pg15" --health-interval 2s --health-timeout 3s --health-retries 15 \
    postgres:15 >/dev/null

docker run -d --name "$PG16" -p 0:5432 \
    -e POSTGRES_USER=user -e POSTGRES_PASSWORD=pass -e POSTGRES_DB=app_pg16 \
    --label "name-route=[{\"protocol\":\"postgres\",\"key\":\"app_pg16\"}]" \
    --health-cmd "pg_isready -U user -d app_pg16" --health-interval 2s --health-timeout 3s --health-retries 15 \
    postgres:16 >/dev/null

docker run -d --name "$PG17" -p 0:5432 \
    -e POSTGRES_USER=user -e POSTGRES_PASSWORD=pass -e POSTGRES_DB=app_pg17 \
    --label "name-route=[{\"protocol\":\"postgres\",\"key\":\"app_pg17\"}]" \
    --health-cmd "pg_isready -U user -d app_pg17" --health-interval 2s --health-timeout 3s --health-retries 15 \
    postgres:17 >/dev/null

docker run -d --name "$MY57" -p 0:3306 \
    -e MYSQL_ALLOW_EMPTY_PASSWORD=yes -e MYSQL_DATABASE=myapp_mysql57 \
    --label "name-route=[{\"protocol\":\"mysql\",\"key\":\"myapp_mysql57\"}]" \
    --health-cmd "mysqladmin ping -h 127.0.0.1" --health-interval 3s --health-timeout 3s --health-retries 30 \
    mysql:5.7 --default-authentication-plugin=mysql_native_password >/dev/null

docker run -d --name "$MY80" -p 0:3306 \
    -e MYSQL_ALLOW_EMPTY_PASSWORD=yes -e MYSQL_DATABASE=myapp_mysql80 \
    --label "name-route=[{\"protocol\":\"mysql\",\"key\":\"myapp_mysql80\"}]" \
    --health-cmd "mysqladmin ping -h 127.0.0.1" --health-interval 3s --health-timeout 3s --health-retries 30 \
    mysql:8.0 --default-authentication-plugin=mysql_native_password >/dev/null

docker run -d --name "$MY84" -p 0:3306 \
    -e MYSQL_ALLOW_EMPTY_PASSWORD=yes -e MYSQL_DATABASE=myapp_mysql84 \
    --label "name-route=[{\"protocol\":\"mysql\",\"key\":\"myapp_mysql84\"}]" \
    --health-cmd "mysqladmin ping -h 127.0.0.1" --health-interval 3s --health-timeout 3s --health-retries 30 \
    mysql:8.4 --mysql-native-password=ON --authentication-policy=mysql_native_password >/dev/null

echo "=== Waiting for healthchecks ==="
wait_healthy_container "$PG14"
wait_healthy_container "$PG15"
wait_healthy_container "$PG16"
wait_healthy_container "$PG17"
wait_healthy_container "$MY57" 90
wait_healthy_container "$MY80" 90
wait_healthy_container "$MY84" 90

# ---- Start nameroute ----
generate_config
echo "=== Starting nameroute ==="
start_nameroute

# ---- Run tests ----
init_results_dir

echo "=== Running DB version tests ==="
echo ""

declare -a PIDS=()
declare -a NAMES=()
declare -a OUTFILES=()

for script in "$SCRIPT_DIR"/*.sh; do
    [ "$(basename "$script")" = "run_all.sh" ] && continue
    name="$(basename "$script" .sh)"
    outfile="$RESULTS_DIR/${name}.out"
    echo "  $name ..."
    PG_PORT="$PG_PORT" MYSQL_PORT="$MYSQL_PORT" bash "$script" > "$outfile" 2>&1 &
    PIDS+=($!)
    NAMES+=("$name")
    OUTFILES+=("$outfile")
done

echo ""
echo "  Waiting for all tests to finish ..."

# ---- Collect results ----
declare -A RESULTS

for i in "${!PIDS[@]}"; do
    wait "${PIDS[$i]}" 2>/dev/null || true
    name="${NAMES[$i]}"
    outfile="${OUTFILES[$i]}"
    # DB version tests produce either PG: or MySQL: line (not both)
    pg=$(grep "^PG:" "$outfile" 2>/dev/null | head -1 | cut -d: -f2 || true)
    my=$(grep "^MySQL:" "$outfile" 2>/dev/null | head -1 | cut -d: -f2 || true)
    if [ -n "$pg" ]; then
        RESULTS[$name]="$pg"
    elif [ -n "$my" ]; then
        RESULTS[$name]="$my"
    else
        RESULTS[$name]="ERROR"
    fi
done

# ---- Display matrix ----
declare -A DB_INFO
DB_INFO[pg14]="PostgreSQL 14|postgres:14"
DB_INFO[pg15]="PostgreSQL 15|postgres:15"
DB_INFO[pg16]="PostgreSQL 16|postgres:16"
DB_INFO[pg17]="PostgreSQL 17|postgres:17"
DB_INFO[mysql57]="MySQL 5.7|mysql:5.7"
DB_INFO[mysql80]="MySQL 8.0|mysql:8.0"
DB_INFO[mysql84]="MySQL 8.4|mysql:8.4"

ORDERED=("pg14" "pg15" "pg16" "pg17" "mysql57" "mysql80" "mysql84")

echo ""
echo "=========================================================================="
echo "  DB Server Version Test Matrix"
echo "=========================================================================="
printf "%-16s | %-20s | %-8s\n" "Server" "Image" "Result"
echo "-----------------|----------------------|--------"

TOTAL_PASS=0
TOTAL_FAIL=0

for name in "${ORDERED[@]}"; do
    IFS='|' read -r server image <<< "${DB_INFO[$name]}"
    res="${RESULTS[$name]:-ERROR}"
    [ "$res" = "PASS" ] && TOTAL_PASS=$((TOTAL_PASS + 1)) || TOTAL_FAIL=$((TOTAL_FAIL + 1))
    printf "%-16s | %-20s | %-8s\n" "$server" "$image" "$res"
done

echo "-----------------|----------------------|--------"
echo ""
echo "=== Results: $TOTAL_PASS passed, $TOTAL_FAIL failed (of $((TOTAL_PASS + TOTAL_FAIL)) tests) ==="

# Show details for failures
HAS_FAILURE=false
for name in "${ORDERED[@]}"; do
    if [ "${RESULTS[$name]:-ERROR}" != "PASS" ]; then
        if ! $HAS_FAILURE; then
            echo ""
            echo "=== Failure details ==="
            HAS_FAILURE=true
        fi
        echo "--- $name ---"
        tail -10 "$RESULTS_DIR/${name}.out" 2>/dev/null || true
        echo ""
    fi
done

if [ "$TOTAL_FAIL" -gt 0 ]; then
    exit 1
fi
