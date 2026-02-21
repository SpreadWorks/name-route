package main

import (
	"context"
	"database/sql"
	"fmt"
	"os"

	_ "github.com/go-sql-driver/mysql"
	_ "github.com/jackc/pgx/v5/stdlib"
)

func main() {
	pgPort := os.Getenv("PG_PORT")
	myPort := os.Getenv("MY_PORT")

	// PG test
	func() {
		dsn := fmt.Sprintf("postgres://user:pass@127.0.0.1:%s/app?sslmode=disable", pgPort)
		db, err := sql.Open("pgx", dsn)
		if err != nil {
			fmt.Printf("PG:FAIL:%v\n", err)
			return
		}
		defer db.Close()
		var v int
		if err := db.QueryRowContext(context.Background(), "SELECT 1").Scan(&v); err != nil || v != 1 {
			fmt.Printf("PG:FAIL:%v\n", err)
			return
		}
		fmt.Println("PG:PASS")
	}()

	// MySQL test
	func() {
		dsn := fmt.Sprintf("root:@tcp(127.0.0.1:%s)/myapp", myPort)
		db, err := sql.Open("mysql", dsn)
		if err != nil {
			fmt.Printf("MySQL:FAIL:%v\n", err)
			return
		}
		defer db.Close()
		var v int
		if err := db.QueryRowContext(context.Background(), "SELECT 1").Scan(&v); err != nil || v != 1 {
			fmt.Printf("MySQL:FAIL:%v\n", err)
			return
		}
		fmt.Println("MySQL:PASS")
	}()
}
