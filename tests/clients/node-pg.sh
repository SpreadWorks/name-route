#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    node:20-slim \
    bash -c 'cd /tmp && npm install --silent pg@8.13.1 mysql2@3.12.0 2>/dev/null && NODE_PATH=/tmp/node_modules node /tests/test_node.js'
