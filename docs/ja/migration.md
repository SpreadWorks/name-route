[English](../migration.md)

# Migration guide

既存のプロジェクトを name-route に移行する手順を説明します。


## Overview

移行のゴールは、プロジェクト内のポート番号のハードコーディングを排除し、名前ベースのルーティングに切り替えることです。

**Before:**
```
http://localhost:3000     → フロントエンド
http://localhost:4000     → API
postgresql://localhost:5432/mydb
```

**After:**
```
http://myapp.localhost:8080       → フロントエンド
http://api.myapp.localhost:8080   → API
postgresql://localhost:15432/mydb
```


## Step 1: Install & start

```bash
# インストール（全プラットフォームの手順は README の Install セクションを参照）
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-unknown-linux-gnu -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/

# 起動（sudo は DNS と /etc/hosts に必要）
sudo nameroute
```


## Step 2: Remove hardcoded ports

### package.json dev scripts

**Before:**
```json
{
  "scripts": {
    "dev": "next dev -p 3000"
  }
}
```

**After:**
```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

`nameroute run` が空きポートを自動割り当てし、`PORT` 環境変数で渡します。Next.js, Vite, Remix などは `PORT` を自動認識します。

### Custom environment variables

プロジェクトが独自の環境変数（例: `DEV_API_PORT`）でポートを管理している場合:

```json
{
  "scripts": {
    "dev:api": "nameroute run http api --port-env DEV_API_PORT -- node server.js"
  }
}
```

### docker-compose.yml

**Before:**
```yaml
services:
  web:
    image: nginx
    ports:
      - "3000:80"
  db:
    image: postgres
    ports:
      - "5432:5432"
```

**After:**
```yaml
services:
  web:
    image: nginx
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":80}]'
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

`ports:` を削除し、`labels:` に `name-route` ラベルを追加します。


## Step 3: Update connection URLs

### Frontend API calls

**Before:**
```javascript
// .env.local
API_URL=http://localhost:4000
```

**After:**
```javascript
// .env.local
API_URL=http://api.myapp.localhost:8080
```

### Database connection

**Before:**
```
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb
```

**After:**
```
DATABASE_URL=postgresql://user:pass@localhost:15432/mydb
```

name-route の PostgreSQL リスナーはデフォルトで `15432` で待ち受けます。データベース名 `mydb` がルーティングキーとして使われます。

### CORS

API サーバーに CORS を設定している場合、`*.localhost` からのリクエストを許可します。

```javascript
// Express の例
app.use(cors({
  origin: /\.localhost(:\d+)?$/
}));
```

```python
# Django の例
CORS_ALLOWED_ORIGIN_REGEXES = [
    r"^https?://.*\.localhost(:\d+)?$",
]
```

```ruby
# Rails の例
Rails.application.config.hosts << /.*\.localhost/
```


## Step 4: Multi-level subdomains

モノレポや複数サービス構成では、マルチレベルサブドメインが便利です。

```bash
# モノレポ構成
nameroute run http myapp -- next dev          # → http://myapp.localhost:8080
nameroute run http api.myapp -- node api.js   # → http://api.myapp.localhost:8080
nameroute run http admin.myapp -- next dev    # → http://admin.myapp.localhost:8080
```

ルーティングキーにドットを含めることで、論理的な階層を表現できます。


## Step 5: Verify routes

```bash
nameroute list
```

```
PROTOCOL     KEY                  BACKEND                  SOURCE   URL
http         myapp                127.0.0.1:43210          run      http://myapp.localhost:8080
http         api.myapp            127.0.0.1:43211          run      http://api.myapp.localhost:8080
postgres     myapp                172.17.0.2:5432          docker
```


## FAQ

### Do I need to change Docker network settings?

いいえ。name-route はコンテナの IP アドレスに直接接続するため、Docker ネットワーク設定を変更する必要はありません。ただし、`ports:` でホストにバインドしていたポートは不要になります。

### Can I run multiple projects simultaneously?

はい。name-route の主要な利点の一つです。各プロジェクトが異なるキーを使えば、ポート衝突なく同時起動できます。

```bash
# プロジェクト A
nameroute run http project-a -- next dev

# プロジェクト B（別ターミナル）
nameroute run http project-b -- next dev
```

### Can I use it without sudo?

はい。`sudo` なしでもルーティング自体は問題なく動作します。主要ブラウザ（Chrome, Firefox, Edge, Safari）は `*.localhost` を自動的に `127.0.0.1` に解決するため、ブラウザからのアクセスであれば DNS サーバーも `/etc/hosts` の編集も不要です。

`sudo` が必要になるのは、`curl` や `wget` などの CLI ツール、あるいは OS のシステムリゾルバに依存するアプリケーションから `*.localhost` にアクセスする場合です。これらのツールはブラウザのような独自の名前解決を持たないため、DNS サーバーか `/etc/hosts` へのエントリが必要になります。

### Can I use it with Supabase?

はい。Supabase の Docker コンテナにラベルを追加し、`API_EXTERNAL_URL` や `SITE_URL` に name-route の URL を設定することで、ポートの管理を不要にできます。
