#!/usr/bin/env bash
set -euo pipefail
MYSQL_PORT="${MYSQL_PORT:-13306}"
docker run --rm --network=host \
    -e MY_PORT="$MYSQL_PORT" \
    python:3.12-slim \
    bash -c '
        pip install -q PyMySQL==1.1.1 2>/dev/null
        python3 -c "
import pymysql, os
port = os.environ[\"MY_PORT\"]
try:
    conn = pymysql.connect(host=\"127.0.0.1\", port=int(port),
                           user=\"root\", password=\"\",
                           database=\"myapp_mysql80\")
    cur = conn.cursor()
    cur.execute(\"SELECT 1\")
    assert cur.fetchone()[0] == 1
    conn.close()
    print(\"MySQL:PASS\")
except Exception as e:
    print(f\"MySQL:FAIL:{e}\")
"'
