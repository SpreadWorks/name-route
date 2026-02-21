#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <libpq-fe.h>

int main() {
    const char *port = getenv("PG_PORT");
    if (!port) port = "15432";

    char conninfo[256];
    snprintf(conninfo, sizeof(conninfo),
        "host=127.0.0.1 port=%s user=user password=pass dbname=app sslmode=disable", port);

    PGconn *conn = PQconnectdb(conninfo);
    if (PQstatus(conn) != CONNECTION_OK) {
        printf("PG:FAIL:%s\n", PQerrorMessage(conn));
        PQfinish(conn);
        return 0;
    }

    PGresult *res = PQexec(conn, "SELECT 1");
    if (PQresultStatus(res) != PGRES_TUPLES_OK || atoi(PQgetvalue(res, 0, 0)) != 1) {
        printf("PG:FAIL\n");
    } else {
        printf("PG:PASS\n");
    }
    PQclear(res);
    PQfinish(conn);
    return 0;
}
