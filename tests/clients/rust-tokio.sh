#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR/rust_test:/tests:ro" \
    rust:1.88 \
    bash -c '
        apt-get update -qq >/dev/null 2>&1
        apt-get install -y -qq pkg-config libssl-dev >/dev/null 2>&1
        cp -r /tests /tmp/dbtest && cd /tmp/dbtest
        cargo run --release 2>&1 | grep -E "^(PG:|MySQL:)|error"
    '
