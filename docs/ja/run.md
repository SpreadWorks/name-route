# nameroute run

`nameroute run` は、開発サーバーの起動とルート登録を一つのコマンドで行います。ポート番号を自動で割り当て、子プロセスに渡し、daemon にルートを登録します。子プロセスが終了すると、ルートは自動で削除されます。


## Basic usage

```bash
nameroute run <protocol> <key> -- <command...>
```

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


## Signal handling

Ctrl+C を押すと、nameroute は子プロセスに SIGINT を転送します。これは通常の Ctrl+C と同じ挙動のため、Node.js や Python などの開発サーバーが graceful shutdown を正しく行えます。子プロセスが終了した後、nameroute は daemon からルートを自動削除します。


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
PROTOCOL     KEY                  BACKEND                  SOURCE   URL
http         myapp                127.0.0.1:43210          run      http://myapp.localhost:8080
postgres     myapp                172.17.0.2:5432          docker
```

HTTP ルートには URL 列が表示され、アクセス先がすぐにわかります。
