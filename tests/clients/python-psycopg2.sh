#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    python:3.12-slim \
    bash -c 'pip install -q psycopg2-binary==2.9.10 PyMySQL==1.1.1 2>/dev/null && python3 /tests/test_python_psycopg2.py'
