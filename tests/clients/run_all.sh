#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

source "$TESTS_DIR/lib/infra.sh"
source "$TESTS_DIR/lib/results.sh"

COMPOSE_PROJECT="nameroute-clients-$$"

cleanup() {
    echo ""
    echo "--- cleanup (clients) ---"
    stop_nameroute
    docker compose -p "$COMPOSE_PROJECT" -f "$TESTS_DIR/docker-compose.yml" down -v 2>/dev/null || true
    cleanup_tmpfiles
    cleanup_results_dir
}
trap cleanup EXIT

# ---- Infrastructure ----
build_nameroute

echo "=== Starting DB containers ==="
docker compose -p "$COMPOSE_PROJECT" -f "$TESTS_DIR/docker-compose.yml" up -d \
    postgres_multi mysql_multi 2>&1 | tail -2

echo "=== Waiting for healthchecks ==="
wait_healthy_compose "$COMPOSE_PROJECT" "$TESTS_DIR/docker-compose.yml" postgres_multi
wait_healthy_compose "$COMPOSE_PROJECT" "$TESTS_DIR/docker-compose.yml" mysql_multi

generate_config
echo "=== Starting nameroute ==="
start_nameroute

# ---- Run tests ----
init_results_dir

echo "=== Running client library tests ==="
echo "  (each language runs in a Docker container with --network=host)"
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
declare -A PG_RESULTS
declare -A MY_RESULTS

for i in "${!PIDS[@]}"; do
    wait "${PIDS[$i]}" 2>/dev/null || true
    name="${NAMES[$i]}"
    readarray -t res < <(collect_result "${OUTFILES[$i]}")
    PG_RESULTS[$name]="${res[0]}"
    MY_RESULTS[$name]="${res[1]}"
done

# ---- Display matrix ----
declare -A PG_LIBS
PG_LIBS[python-psycopg2]="psycopg2 2.9.10"
PG_LIBS[python-psycopg3]="psycopg 3.2.4"
PG_LIBS[node-pg]="pg 8.13.1"
PG_LIBS[ruby-pg]="pg 1.5.9"
PG_LIBS[php-pdo]="PDO pgsql"
PG_LIBS[go-pgx]="pgx 5.7.2"
PG_LIBS[java-jdbc]="JDBC postgresql 42.7.4"
PG_LIBS[c-libpq]="libpq"
PG_LIBS[rust-tokio]="tokio-postgres 0.7.12"

declare -A MY_LIBS
MY_LIBS[python-psycopg2]="PyMySQL 1.1.1"
MY_LIBS[python-psycopg3]="mysqlclient 2.2.7"
MY_LIBS[node-pg]="mysql2 3.12.0"
MY_LIBS[ruby-pg]="mysql2 0.5.6"
MY_LIBS[php-pdo]="PDO mysql"
MY_LIBS[go-pgx]="go-sql-driver 1.8.1"
MY_LIBS[java-jdbc]="mysql-connector-j 9.1.0"
MY_LIBS[c-libpq]="libmysqlclient"
MY_LIBS[rust-tokio]="mysql_async 0.34.2"

ORDERED=("python-psycopg2" "python-psycopg3" "node-pg" "ruby-pg" "php-pdo" "go-pgx" "java-jdbc" "c-libpq" "rust-tokio")

echo ""
echo "=========================================================================="
echo "  Client Library Test Matrix"
echo "=========================================================================="
printf "%-18s | %-24s | %-6s | %-24s | %-6s\n" "Client" "PG Library" "PG" "MySQL Library" "MySQL"
echo "-------------------|--------------------------|--------|--------------------------|-------"

TOTAL_PASS=0
TOTAL_FAIL=0

for name in "${ORDERED[@]}"; do
    pg_res="${PG_RESULTS[$name]:-ERROR}"
    my_res="${MY_RESULTS[$name]:-ERROR}"
    pg_lib="${PG_LIBS[$name]:-?}"
    my_lib="${MY_LIBS[$name]:-?}"

    [ "$pg_res" = "PASS" ] && TOTAL_PASS=$((TOTAL_PASS + 1)) || TOTAL_FAIL=$((TOTAL_FAIL + 1))
    [ "$my_res" = "PASS" ] && TOTAL_PASS=$((TOTAL_PASS + 1)) || TOTAL_FAIL=$((TOTAL_FAIL + 1))

    printf "%-18s | %-24s | %-6s | %-24s | %-6s\n" "$name" "$pg_lib" "$pg_res" "$my_lib" "$my_res"
done

echo "-------------------|--------------------------|--------|--------------------------|-------"
echo ""
echo "=== Results: $TOTAL_PASS passed, $TOTAL_FAIL failed (of $((TOTAL_PASS + TOTAL_FAIL)) tests) ==="

# Show details for failures
HAS_FAILURE=false
for name in "${ORDERED[@]}"; do
    if [ "${PG_RESULTS[$name]:-ERROR}" != "PASS" ] || [ "${MY_RESULTS[$name]:-ERROR}" != "PASS" ]; then
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
