<?php
$pgPort = getenv('PG_PORT');
$myPort = getenv('MY_PORT');

try {
    $pdo = new PDO("pgsql:host=127.0.0.1;port=$pgPort;dbname=app;sslmode=disable",
                   'user', 'pass');
    $stmt = $pdo->query('SELECT 1');
    $val = $stmt->fetchColumn();
    if ($val != 1) throw new Exception('unexpected');
    echo "PG:PASS\n";
} catch (Exception $e) {
    echo "PG:FAIL:" . $e->getMessage() . "\n";
}

try {
    $pdo = new PDO("mysql:host=127.0.0.1;port=$myPort;dbname=myapp", 'root', '');
    $stmt = $pdo->query('SELECT 1');
    $val = $stmt->fetchColumn();
    if ($val != 1) throw new Exception('unexpected');
    echo "MySQL:PASS\n";
} catch (Exception $e) {
    echo "MySQL:FAIL:" . $e->getMessage() . "\n";
}
