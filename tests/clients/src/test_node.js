const { Client } = require("pg");
const mysql = require("mysql2/promise");

const PG_PORT = process.env.PG_PORT;
const MY_PORT = process.env.MY_PORT;

async function main() {
  try {
    const client = new Client({
      host: "127.0.0.1", port: PG_PORT,
      user: "user", password: "pass", database: "app",
      ssl: false,
    });
    await client.connect();
    const res = await client.query("SELECT 1 AS v");
    if (res.rows[0].v !== 1) throw new Error("unexpected");
    await client.end();
    console.log("PG:PASS");
  } catch (e) {
    console.log("PG:FAIL:" + e.message);
  }

  try {
    const conn = await mysql.createConnection({
      host: "127.0.0.1", port: MY_PORT,
      user: "root", password: "", database: "myapp",
    });
    const [rows] = await conn.query("SELECT 1 AS v");
    if (rows[0].v !== 1) throw new Error("unexpected");
    await conn.end();
    console.log("MySQL:PASS");
  } catch (e) {
    console.log("MySQL:FAIL:" + e.message);
  }
}
main();
