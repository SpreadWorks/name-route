#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    gcc:13 \
    bash -c '
        set -e
        apt-get update -qq >/dev/null 2>&1
        apt-get install -y -qq libpq-dev default-libmysqlclient-dev >/dev/null 2>&1
        set +e
        gcc /tests/test_pg.c -o /tmp/test_pg -I$(pg_config --includedir) -L$(pg_config --libdir) -lpq 2>&1
        gcc /tests/test_mysql.c -o /tmp/test_mysql $(mysql_config --cflags --libs) 2>&1
        [ -x /tmp/test_pg ] && /tmp/test_pg || echo "PG:FAIL:compilation failed"
        [ -x /tmp/test_mysql ] && /tmp/test_mysql || echo "MySQL:FAIL:compilation failed"
    '
