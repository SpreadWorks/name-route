[English](../run.md)

# nameroute run

`nameroute run` は、開発サーバーの起動とルート登録を一つのコマンドで行います。ポート番号を自動で割り当て、子プロセスに渡し、daemon にルートを登録します。子プロセスが終了すると、ルートは自動で削除されます。


## Basic usage

```bash
nameroute run <protocol> <key> -- <command...>
```

### エイリアス

`--alias` は繰り返し指定でき、同じ子プロセス・同じバックエンドポートを複数の route key で公開します。全 key は原子的に予約されるため、競合時に一部だけ登録されることはありません。

```bash
nameroute run http app --alias api --alias admin -- next dev
```

これらの route はこの起動だけの owner に紐付き、終了時にまとめて削除されます。後から追加された手動・検出 route を古い cleanup が削除することはありません。

```bash
# Next.js の開発サーバーを起動
nameroute run http myapp -- next dev

# → 空きポートが自動割り当てされ、PORT 環境変数で next dev に渡される
# → http://myapp.localhost:8080 でアクセス可能
# → Ctrl+C で停止 & ルート自動削除
```


## Port passing

### PORT environment variable (default)

`nameroute run` は空きポートを確保し、子プロセスに `PORT` 環境変数として渡します。Next.js, Vite, Rails など多くのフレームワークは `PORT` 環境変数を自動的に認識します。

```bash
nameroute run http myapp -- next dev
# next dev は PORT=XXXXX を受け取って起動
```

### $PORT argument substitution

コマンド引数中の `$PORT` は、割り当てられたポート番号に自動で置換されます。`PORT` 環境変数に対応していないコマンドで使えます。

```bash
nameroute run http myapp -- python3 -m http.server '$PORT'
# → python3 -m http.server 12345 のように展開される
```

> **Note:** シェルが `$PORT` を展開しないよう、シングルクォートで囲んでください。

### --port-env option

`PORT` 以外の環境変数名でポートを渡したい場合に使います。`PORT` に加えて、指定した名前の環境変数も設定されます。

```bash
nameroute run http api --port-env DEV_API_PORT -- next dev
# → PORT=XXXXX と DEV_API_PORT=XXXXX の両方がセットされる
```

複数サービスが独自の環境変数名を期待している場合に便利です。


## --detect-port mode

`PORT` 環境変数に対応していないコマンドや、自分でポートを決めるコマンドの場合、`--detect-port` を使うと stdout/stderr からポートを自動検出します。

```bash
nameroute run http myapp --detect-port -- python3 -m http.server 0
# → stdout の "http://0.0.0.0:XXXXX" からポートを検出してルート登録
```

検出対象のパターン:
```
http://localhost:<port>
http://127.0.0.1:<port>
http://0.0.0.0:<port>
https://localhost:<port>
```

### FORCE_COLOR

`--detect-port` モードでは stdout/stderr がパイプ経由になるため、子プロセスがカラー出力を無効化することがあります。nameroute は `FORCE_COLOR=1` 環境変数を自動設定し、カラー出力を維持します。


## HTTPS with --tls-mode

`--tls-mode terminate` を指定すると、name-route が TLS を終端する HTTPS ルートを登録できます。バックエンドは HTTP のまま動作します。

```bash
nameroute run https myapp --tls-mode terminate -- next dev --port '$PORT'
```

daemon の設定ファイルに `[tls]` セクション（証明書・鍵のパス、例: `/etc/nameroute/cert.pem`, `/etc/nameroute/key.pem`）が必要です。セットアップは [HTTPS](https.md) を参照してください。

`--tls-mode` を省略すると passthrough モードになり、バックエンド自身が TLS を処理する必要があります。


## Shutdown behavior

Linux/macOS では子プロセスを専用 process group で管理します。`nameroute run` が受けた SIGINT/SIGTERM は同じ種類でその group に送られ、猶予時間を超えて残るプロセスは SIGKILL で終了します。route cleanup には timeout があり、owner 一致時だけを削除するため、置換済み route を削除したり無期限待機したりしません。

TTY を使う場合は子 group を foreground に渡し、終了後は cleanup より前に呼出元の process group へ戻します。空き port 用 listener は spawn 直前まで保持しますが、任意の子コマンドへ listener FD を移譲する方法は移植性がないため、最後の bind-to-exec 区間を完全に予約することはできません。

遅延した management 接続で route が復活しないよう、daemon は終了済み `run` の owner UUID tombstone を daemon 再起動まで保持します。これにより遅延 register は no-op になります。メモリは完了した run 数に応じて増え、daemon 再起動時に解放されます。

明示的に別の process group や session へ離脱した子孫は、この停止保証の対象外です。対話実行時だけ child group へ terminal を渡し、background の `nameroute run ... &` は shell から terminal を奪いません。


## package.json example

```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev",
    "dev:api": "nameroute run http api.myapp -- node server.js"
  }
}
```

マルチレベルサブドメイン（`api.myapp`）を使えば、`http://api.myapp.localhost:8080` のような URL でアクセスできます。

モノレポや HTTPS 構成を含むその他の例は [構成例](examples.md) を参照してください。


## docker-compose.yml example

Docker コンテナ内のプロセスではなく、ホスト側で動かす開発サーバーに使います。

```yaml
# docker-compose.yml (DB のみ)
services:
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

```json
// package.json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

```bash
docker compose up -d   # DB 起動
npm run dev            # アプリ起動
# → http://myapp.localhost:8080 でアクセス
# → psql -h localhost -p 15432 -d myapp で DB 接続
```


## Route listing

```bash
nameroute list
```

```
PROTOCOL     KEY                  BACKEND                  SOURCE   HEALTH     URL
http         myapp                127.0.0.1:43210          run      healthy    http://myapp.localhost:8080
postgres     myapp                172.17.0.2:5432          docker   healthy
```

HEALTH 列でバックエンドの接続状態を確認できます。
