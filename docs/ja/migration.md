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
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-unknown-linux-musl -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/

# 起動（sudo は /etc/hosts の管理に必要）
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

はい。`sudo` なしでもルーティング自体は問題なく動作します。主要ブラウザ（Chrome, Firefox, Edge, Safari）は `*.localhost` を自動的に `127.0.0.1` に解決するため、ブラウザからのアクセスであれば `/etc/hosts` の編集も不要です。

`sudo` が必要になるのは、`curl` や `wget` などの CLI ツール、あるいはサーバー間通信など OS のリゾルバに依存するアプリケーションから `*.localhost` にアクセスする場合です。これらは `/etc/hosts` へのエントリが必要になります。

### Can I use it with Supabase?

はい。Supabase の Docker コンテナにラベルを追加し、`API_EXTERNAL_URL` や `SITE_URL` に name-route の URL を設定することで、ポートの管理を不要にできます。


---

## AI エージェント向け指示書

このセクションは、AI エージェントが既存プロジェクトの name-route 移行を行うための指示書です。

AI に以下のプロンプトをコピペして使ってください:

```
以下の URL に記載されている手順に従って、このプロジェクトに name-route を導入してください。
https://raw.githubusercontent.com/SpreadWorks/name-route/refs/heads/main/docs/ja/migration.md

対象のプロジェクトディレクトリ: /path/to/project
```

### 前提条件

- `nameroute` はシステムにインストール済み。
- デーモンは systemctl で管理する（`sudo systemctl start nameroute`）。systemctl が使えない環境の場合は、nameroute の起動・停止方法をユーザーに確認すること。
- `nameroute status` でデーモンが起動中であることを確認する。

### デフォルトのリスナーポート

| プロトコル  | アドレス              | ルーティングキーの取得元   |
|------------|---------------------|--------------------------|
| HTTP       | `127.0.0.1:8080`    | Host ヘッダーのサブドメイン |
| HTTPS      | `127.0.0.1:8443`    | SNI (TLS Server Name)    |
| PostgreSQL | `127.0.0.1:15432`   | データベース名             |
| MySQL      | `127.0.0.1:13306`   | データベース名             |
| SMTP       | `127.0.0.1:10025`   | 宛先ドメイン               |

### 移行パターン

#### Pattern 1: ホスト側の dev server (`nameroute run`)

起動コマンドを `nameroute run` でラップする。`nameroute run` は空きポートを自動割り当てし、`PORT` 環境変数で子プロセスに渡す。Next.js, Vite, Rails 等は `PORT` を自動認識する。

```json
{ "scripts": { "dev": "nameroute run http myapp -- next dev" } }
```

バリエーション:
- **`$PORT` 置換**（`PORT` 環境変数を読まないコマンド向け）: `nameroute run http myapp -- python3 -m http.server '$PORT'`
- **`--port-env`**（追加の環境変数）: `nameroute run http myapp --port-env DEV_API_PORT -- node server.js`
- **`--detect-port`**（stdout/stderr からポートを自動検出）: `nameroute run http myapp --detect-port -- some-server`

#### Pattern 2: Docker コンテナ（ラベル）

`ports:` を削除し、`name-route` ラベルを追加する。ラベルの値は JSON 配列:

```yaml
labels:
  name-route: '[{"protocol":"http","key":"myapp","port":80}]'
```

| フィールド  | 必須 | デフォルト                       |
|------------|------|--------------------------------|
| `protocol` | Yes  |                                |
| `key`      | No   | コンテナ名                      |
| `port`     | No   | HTTP=80, HTTPS=443, PG=5432, MySQL=3306, SMTP=25 |

#### Pattern 3: ホスト側アプリ + Docker DB（混在）

アプリに Pattern 1、データベースに Pattern 2 を適用する。上記の移行手順の例を参照。

#### Pattern 4: マルチレベルサブドメイン（モノレポ）

ドット付きのキーを使う: `frontend.myproject`, `api.myproject` → `http://frontend.myproject.localhost:8080`, `http://api.myproject.localhost:8080`

#### Pattern 5: HTTPS (TLS 終端)

`--tls-mode terminate` を使う。デーモンの設定に `[tls]` の cert/key が必要。マルチレベルサブドメインの場合は `/etc/nameroute/domains` から証明書を再生成する。

### DATABASE_URL のルール

- **ホスト→DB**: ポートを nameroute のリスナーポートに変更する（PostgreSQL: 15432, MySQL: 13306）。データベース名がルーティングキーになる。
- **Docker 内部通信**: コンテナが Docker ネットワーク経由でサービス名をホスト名として接続している場合（例: `mysql://dbuser:secret@mysql/eccubedb`）は変更**しない**。
- **BaaS (Supabase, Firebase 等)**: 独自 CLI（例: `npx supabase start`）で管理され、SDK（`SUPABASE_URL`, `SUPABASE_ANON_KEY` 等）で接続している場合は、BaaS の接続設定は変更しない。HTTP アプリサーバー（Next.js, Vite 等）にのみ `nameroute run` を適用する。

### 移行チェックリスト

1. **デーモンの起動を確認** — `nameroute status`
2. **サービスを特定** — プロジェクトが使う dev server とデータベースをすべてリストアップ
3. **ルーティングキーを決定** — 通常はプロジェクト名（例: `myapp`）
4. **ホスト側 dev server** — 起動コマンドを `nameroute run http <key> -- <command>` でラップ
5. **Docker サービス** — `ports:` を `name-route` ラベルに置き換え
6. **DATABASE_URL を更新** — ポートを nameroute のリスナーポートに変更（15432, 13306）。上記の DATABASE_URL ルールの例外に注意。
7. **ハードコード URL を更新** — `http://localhost:<port>` を `http://<key>.localhost:8080` に変更する。CORS 設定（`Access-Control-Allow-Origin`, `CORS_ALLOW_ORIGIN` 等）や、オリジンに依存する設定（`TRUSTED_HOSTS`, CSP ヘッダー等）も `*.localhost` を許可するよう更新する。
8. **HTTPS が必要な場合** — `--tls-mode terminate` を使い、デーモンの `[tls]` 設定を確認
9. **テスト** — `nameroute list` でルートを確認し、名前でアクセスできることを検証
