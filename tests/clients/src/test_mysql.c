#include <stdio.h>
#include <stdlib.h>
#include <mysql/mysql.h>

int main() {
    const char *port_str = getenv("MY_PORT");
    if (!port_str) port_str = "13306";
    int port = atoi(port_str);

    MYSQL *conn = mysql_init(NULL);
    if (!mysql_real_connect(conn, "127.0.0.1", "root", "", "myapp",
                            port, NULL, 0)) {
        printf("MySQL:FAIL:%s\n", mysql_error(conn));
        mysql_close(conn);
        return 0;
    }

    if (mysql_query(conn, "SELECT 1")) {
        printf("MySQL:FAIL:%s\n", mysql_error(conn));
    } else {
        MYSQL_RES *result = mysql_store_result(conn);
        MYSQL_ROW row = mysql_fetch_row(result);
        if (row && atoi(row[0]) == 1) {
            printf("MySQL:PASS\n");
        } else {
            printf("MySQL:FAIL\n");
        }
        mysql_free_result(result);
    }
    mysql_close(conn);
    return 0;
}
