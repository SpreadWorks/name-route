#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
MYSQL_PORT="${MYSQL_PORT:-13306}"
SRC_DIR="$(cd "$(dirname "$0")/src" && pwd)"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" -e MY_PORT="$MYSQL_PORT" \
    -v "$SRC_DIR:/tests:ro" \
    eclipse-temurin:21 \
    bash -c '
        PG_JAR=https://repo1.maven.org/maven2/org/postgresql/postgresql/42.7.4/postgresql-42.7.4.jar
        MY_JAR=https://repo1.maven.org/maven2/com/mysql/mysql-connector-j/9.1.0/mysql-connector-j-9.1.0.jar
        apt-get update -qq && apt-get install -y -qq wget >/dev/null 2>&1
        wget -q -O /tmp/pg.jar "$PG_JAR"
        wget -q -O /tmp/my.jar "$MY_JAR"
        cp /tests/Test.java /tmp/Test.java
        javac -cp "/tmp/pg.jar:/tmp/my.jar" /tmp/Test.java -d /tmp
        java -cp "/tmp:/tmp/pg.jar:/tmp/my.jar" Test
    '
