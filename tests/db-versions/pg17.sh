#!/usr/bin/env bash
set -euo pipefail
PG_PORT="${PG_PORT:-15432}"
docker run --rm --network=host \
    -e PG_PORT="$PG_PORT" \
    python:3.12-slim \
    bash -c '
        pip install -q psycopg2-binary==2.9.10 2>/dev/null
        python3 -c "
import psycopg2, os
port = os.environ[\"PG_PORT\"]
try:
    conn = psycopg2.connect(host=\"127.0.0.1\", port=int(port),
                            user=\"user\", password=\"pass\",
                            dbname=\"app_pg17\", sslmode=\"disable\")
    cur = conn.cursor()
    cur.execute(\"SELECT 1\")
    assert cur.fetchone()[0] == 1
    conn.close()
    print(\"PG:PASS\")
except Exception as e:
    print(f\"PG:FAIL:{e}\")
"'
