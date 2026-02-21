use std::env;

#[tokio::main]
async fn main() {
    let pg_port = env::var("PG_PORT").unwrap_or_else(|_| "15432".into());
    let my_port = env::var("MY_PORT").unwrap_or_else(|_| "13306".into());

    // PG test
    match test_pg(&pg_port).await {
        Ok(_) => println!("PG:PASS"),
        Err(e) => println!("PG:FAIL:{e}"),
    }

    // MySQL test
    match test_mysql(&my_port).await {
        Ok(_) => println!("MySQL:PASS"),
        Err(e) => println!("MySQL:FAIL:{e}"),
    }
}

async fn test_pg(port: &str) -> Result<(), Box<dyn std::error::Error>> {
    let connstr = format!(
        "host=127.0.0.1 port={port} user=user password=pass dbname=app sslmode=disable"
    );
    let (client, connection) = tokio_postgres::connect(&connstr, tokio_postgres::NoTls).await?;
    tokio::spawn(async move { connection.await.ok(); });
    let row = client.query_one("SELECT 1::int4", &[]).await?;
    let val: i32 = row.get(0);
    assert_eq!(val, 1);
    Ok(())
}

async fn test_mysql(port: &str) -> Result<(), Box<dyn std::error::Error>> {
    use mysql_async::prelude::*;
    let url = format!("mysql://root@127.0.0.1:{port}/myapp");
    let pool = mysql_async::Pool::new(url.as_str());
    let mut conn = pool.get_conn().await?;
    let val: Option<i32> = conn.query_first("SELECT 1").await?;
    assert_eq!(val, Some(1));
    drop(conn);
    pool.disconnect().await?;
    Ok(())
}
