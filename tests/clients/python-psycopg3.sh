#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    python:3.12-slim \
    bash -c 'apt-get update -qq && apt-get install -y -qq pkg-config default-libmysqlclient-dev build-essential >/dev/null 2>&1 && pip install -q "psycopg[binary]==3.2.4" mysqlclient==2.2.7 2>/dev/null && python3 /tests/test_python_psycopg3.py'
