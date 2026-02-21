import os

pg_port = os.environ["PG_PORT"]
my_port = os.environ["MY_PORT"]

try:
    import psycopg
    conn = psycopg.connect(
        f"host=127.0.0.1 port={pg_port} user=user password=pass dbname=app sslmode=disable"
    )
    cur = conn.cursor()
    cur.execute("SELECT 1")
    assert cur.fetchone()[0] == 1
    conn.close()
    print("PG:PASS")
except Exception as e:
    print(f"PG:FAIL:{e}")

try:
    import MySQLdb
    conn = MySQLdb.connect(host="127.0.0.1", port=int(my_port),
                           user="root", passwd="", db="myapp")
    cur = conn.cursor()
    cur.execute("SELECT 1")
    assert cur.fetchone()[0] == 1
    conn.close()
    print("MySQL:PASS")
except Exception as e:
    print(f"MySQL:FAIL:{e}")
