#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    php:8.3-cli \
    bash -c 'apt-get update -qq && apt-get install -y -qq libpq-dev >/dev/null 2>&1 && docker-php-ext-install -j$(nproc) pdo_pgsql pdo_mysql >/dev/null 2>&1 && php /tests/test_php.php'
