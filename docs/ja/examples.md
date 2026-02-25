[English](../examples.md)

# 構成例

このページでは、よくあるプロジェクト構成と name-route の設定方法を紹介します。


## 1. 単一アプリ — `nameroute run`

最もシンプルな構成。開発サーバー1つにルート1つ。

```
myapp/
├── package.json
└── ...
```

```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

```bash
npm run dev
# → http://myapp.localhost:8080
```

ルート設定方法: **`nameroute run`** — ポートは自動割り当てされ、プロセス終了時にルートも自動削除されます。


## 2. アプリ + DB — `nameroute run` + Docker label

ホスト側で Next.js、Docker で PostgreSQL を動かす構成。

```
myapp/
├── docker-compose.yml
├── package.json
├── .env
└── ...
```

```yaml
# docker-compose.yml
services:
  db:
    image: postgres
    environment:
      POSTGRES_DB: myapp
      POSTGRES_USER: myapp
      POSTGRES_PASSWORD: myapp
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

```bash
# .env
DATABASE_URL=postgres://myapp:myapp@localhost:15432/myapp
```

```bash
docker compose up -d   # DB 起動
npm run dev            # アプリ起動

# → http://myapp.localhost:8080  (アプリ)
# → psql -h localhost -p 15432 -d myapp  (DB)
```

ルート設定方法: アプリは **`nameroute run`**、データベースは **Docker label**。`docker-compose.yml` に `ports:` は不要です。


## 3. モノレポ (HTTPS) — `nameroute run`

Next.js フロントエンドと Wrangler (Cloudflare Workers) API で構成されるモノレポ。TLS terminate で全サービスを HTTPS 化。

```
myproject/
├── apps/
│   ├── frontend/   # Next.js  → https://frontend.myproject.localhost:8443
│   ├── backend/    # Vite     → https://backend.myproject.localhost:8443
│   └── api/        # Wrangler → https://api.myproject.localhost:8443
└── package.json
```

各アプリの `package.json`:

```jsonc
// apps/frontend/package.json — Next.js は PORT を自動認識
{
  "scripts": {
    "dev": "nameroute run https frontend.myproject --tls-mode terminate -- next dev"
  }
}

// apps/backend/package.json — Vite は明示的に --port が必要
{
  "scripts": {
    "dev": "nameroute run https backend.myproject --tls-mode terminate -- vite dev --host --port '$PORT'"
  }
}

// apps/api/package.json — Wrangler は $PORT 展開のため sh -c が必要
{
  "scripts": {
    "dev": "nameroute run https api.myproject --tls-mode terminate -- sh -c 'CI=1 wrangler dev src/index.ts --ip 0.0.0.0 --port \"$PORT\"'"
  }
}
```

最初の `*.myproject` ルートが登録されると、daemon は `*.myproject.localhost` を `/etc/nameroute/domains` に自動追記し、証明書再生成コマンドをログに出力します。以下を実行して証明書を更新してください:

```bash
sudo xargs mkcert \
  -key-file /etc/nameroute/key.pem \
  -cert-file /etc/nameroute/cert.pem \
  < /etc/nameroute/domains
sudo systemctl restart nameroute
```

証明書の初期セットアップは [HTTPS — Terminate mode](https.md#terminate-mode) を参照してください。

ルート設定方法: **`nameroute run`** でマルチレベルサブドメインキーと `--tls-mode terminate` を使用。


## 4. Full Docker — Docker label のみ

全サービスを Docker コンテナで実行。`ports:` は不要 — name-route が名前でルーティングします。

```yaml
# docker-compose.yml
services:
  web:
    build: .
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":3000}]'

  api:
    build: ./api
    labels:
      name-route: '[{"protocol":"http","key":"api.myapp","port":8000}]'

  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'

  mail:
    image: mailhog/mailhog
    labels:
      name-route: |
        [
          {"protocol":"http","key":"mail.myapp","port":8025},
          {"protocol":"smtp","key":"myapp","port":1025}
        ]
```

```bash
docker compose up -d

# → http://myapp.localhost:8080       (web)
# → http://api.myapp.localhost:8080   (API)
# → http://mail.myapp.localhost:8080  (メール UI)
# → psql -h localhost -p 15432 -d myapp  (DB)
# → smtp://localhost:10025 key=myapp  (SMTP)
```

ルート設定方法: **Docker label** — name-route が Docker をポーリングしてルートを自動登録。ポートマッピング不要、プロジェクト間のポート衝突なし。ラベルの詳細は [Docker integration](docker.md) を参照してください。


## 5. Discovery — `.nameroute.toml`

ワークスペースディレクトリ配下の複数プロジェクトを自動検出。各リポジトリに `.nameroute.toml` を配置すると、daemon が自動的にルートを登録します。

```
~/workspace/
├── blog/
│   ├── .nameroute.toml
│   └── ...
├── shop/
│   ├── .nameroute.toml
│   └── ...
└── api-server/
    ├── .nameroute.toml
    └── ...
```

各プロジェクトの `.nameroute.toml`:

```toml
# ~/workspace/blog/.nameroute.toml
[[routes]]
protocol = "http"
key = "blog"
backend = "127.0.0.1:3000"
```

```toml
# ~/workspace/shop/.nameroute.toml
[[routes]]
protocol = "http"
key = "shop"
backend = "127.0.0.1:3001"
```

```toml
# ~/workspace/api-server/.nameroute.toml
[[routes]]
protocol = "http"
key = "api"
backend = "127.0.0.1:8000"
```

daemon の設定:

```toml
[discovery]
enabled = true
paths = ["~/workspace"]
poll_interval = 3
```

```bash
# → http://blog.localhost:8080
# → http://shop.localhost:8080
# → http://api.localhost:8080
```

ルート設定方法: **Discovery** — daemon が設定されたディレクトリを定期スキャンし、`.nameroute.toml` ファイルを検出します。ファイルの追加・削除に応じてルートが自動で登録・解除されます。`.nameroute.toml` はバージョン管理にコミットできるため、チーム全員が同じルーティング設定を共有できます。


## ルート設定方法まとめ

| 方法 | 仕組み | 適したケース |
|------|--------|-------------|
| `nameroute run` | 開発コマンドをラップし、ポート割り当て・ルート登録を自動化 | ホスト側の開発サーバー |
| Docker label | コンテナの `name-route` ラベルを自動検出 | Docker 化されたサービス |
| Discovery | プロジェクトディレクトリの `.nameroute.toml` を自動スキャン | 複数プロジェクトのワークスペース |
| Static route | daemon 設定ファイルの `[[routes]]` | 常時稼働のサービス |
| `nameroute add` | CLI コマンドで手動登録 | 一時的な用途・スクリプト |
