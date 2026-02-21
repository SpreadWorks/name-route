#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    ruby:3.3-slim \
    bash -c 'apt-get update -qq && apt-get install -y -qq libpq-dev default-libmysqlclient-dev build-essential >/dev/null 2>&1 && gem install "pg:1.5.9" "mysql2:0.5.6" --no-document >/dev/null 2>&1 && ruby /tests/test_ruby.rb'
