#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -e GOFLAGS=-buildvcs=false \
    -v "$SRC_DIR:/tests:ro" \
    golang:1.22 \
    bash -c 'mkdir -p /tmp/gotest && cp /tests/test_go.go /tmp/gotest/main.go && cp /tests/test_go_mod /tmp/gotest/go.mod && cd /tmp/gotest && go mod tidy 2>/dev/null && go run main.go'
