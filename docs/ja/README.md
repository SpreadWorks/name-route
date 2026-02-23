# name-route

> Name it. We route it.

複数プロジェクトの同時開発や、AIによる並列開発では、
管理するポートが増え続けます。

name-route は、ポート番号の代わりに「名前」でアクセスできるようにする
ローカル開発専用のルーティングツールです。

覚えるポートはプロトコルごとに一つだけ。
あとはデータベース名やホスト名を指定するだけで、適切なサービスに自動でつながります。

> **Note:** name-route はローカル環境での利用を前提に設計されています。
> 外部公開やプロダクション環境での使用は想定していません。

## How it works

<p align="center">
  <img src="../images/how-it-works.svg" alt="How it works" width="800" />
</p>

プロトコルごとに1つのポートを覚えるだけで、あとは名前で振り分けます。

- **HTTP** — `http://A.localhost:8080` のように、サブドメインがルーティングキーになる
- **PostgreSQL / MySQL** — 接続先のデータベース名がルーティングキーになる
- **SMTP** — `smtp://B.localhost:10025` のように、宛先ドメインがルーティングキーになる

バックエンドの実ポートが何番でも、開発者が意識するのは名前だけです。

## Features

- **4つのプロトコル対応** — HTTP, PostgreSQL, MySQL, SMTP をプロトコルレベルで解析してルーティング
- **HTTPS (passthrough / terminate)** — デフォルトは passthrough（TLS をバックエンドにそのまま転送、証明書不要）。terminate モードでは mkcert 等の証明書で TLS を終端し、バックエンドに HTTP で転送
- **WebSocket 透過** — HTTP プロキシは `copy_bidirectional` による透過リレーのため、WebSocket（Next.js HMR 等）がそのまま動作
- **`nameroute run`** — 空きポート自動割り当て + ルート自動登録。ポート番号の管理から解放
- **Docker 自動検出** — コンテナのラベルからルートを自動登録。起動・停止に追従
- **Discovery** — プロジェクトごとの `.nameroute.toml` を自動検出。Git 管理可能
- **静的ルート** — TOML 設定ファイルで Docker を使わないサービスも登録可能
- **マルチレベルサブドメイン** — `api.myapp.localhost` のような多段サブドメインに対応
- **設定なしで起動** — デフォルトで全プロトコルのリスナーが立ち上がる。設定ファイルは任意
- **内蔵 DNS サーバー** — `*.localhost` を自動で `127.0.0.1` に解決（root 時。ブラウザのみなら不要）
- **/etc/hosts 自動管理** — HTTP ルートに対応するホスト名を自動で追加・削除（root 時。ブラウザのみなら不要）
- **シングルバイナリ** — 依存なし。1ファイルを置くだけで動作

## Install

### Homebrew (macOS / Linux)

```bash
brew install SpreadWorks/tap/nameroute
```

### deb (Debian / Ubuntu)

```bash
# x86_64
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute_amd64.deb
sudo dpkg -i nameroute_amd64.deb

# ARM64
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute_arm64.deb
sudo dpkg -i nameroute_arm64.deb
```

### rpm (RHEL / Fedora)

```bash
# x86_64
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64.rpm
sudo rpm -i nameroute-x86_64.rpm

# ARM64
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-aarch64.rpm
sudo rpm -i nameroute-aarch64.rpm
```

### バイナリを直接ダウンロード

```bash
# macOS (Apple Silicon)
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-aarch64-apple-darwin -o nameroute

# macOS (Intel)
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-apple-darwin -o nameroute

# Linux (x86_64)
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-unknown-linux-gnu -o nameroute

# Linux (ARM64)
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-aarch64-unknown-linux-gnu -o nameroute
```

```bash
chmod +x nameroute
sudo mv nameroute /usr/local/bin/
```

### ソースからビルド

```bash
cargo install --git https://github.com/SpreadWorks/name-route
```

## Quick Start

### 1. 起動する

```bash
sudo nameroute
```

設定ファイルなしで、全プロトコルのリスナーが起動します。

> **`sudo` なしでも動作します。**
> 主要ブラウザ（Chrome, Firefox, Edge, Safari）は `*.localhost` を自動的に `127.0.0.1` に解決するため、
> ブラウザからのアクセスだけなら DNS や `/etc/hosts` の編集は不要です。
>
> `sudo` が必要になるのは、`curl` や `wget` などの CLI ツール、
> あるいはシステムのリゾルバを使うアプリケーションから `*.localhost` にアクセスする場合です。
> これらは OS の名前解決に依存するため、DNS サーバーか `/etc/hosts` へのエントリが必要になります。

### 2. ルートを登録する

**Docker の場合** — コンテナにラベルを付けるだけで自動登録されます。

```yaml
# docker-compose.yml
services:
  web:
    image: nginx
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":3000}]'
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

**`nameroute run` の場合** — ポート割り当てからルート登録まで全自動で行います。

```bash
nameroute run http myapp -- next dev
# → 空きポートが自動割り当てされ、http://myapp.localhost:8080 でアクセス可能に
# → Ctrl+C で子プロセス停止 & ルート自動削除
```

```json
// package.json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

ポート番号を完全に意識する必要がなくなります。詳細は [run コマンドガイド](run.md) を参照してください。

**コマンドの場合** — `add` で動的にルートを追加できます。

```bash
nameroute add http myapp 127.0.0.1:3000
nameroute add postgres myapp 127.0.0.1:5432
```

**設定ファイルの場合** — TOML で静的ルートを定義します。

```bash
sudo nameroute --config routes.toml
```

```toml
# routes.toml
[[routes]]
protocol = "http"
key = "myapp"
backend = "127.0.0.1:3000"

[[routes]]
protocol = "postgres"
key = "myapp"
backend = "127.0.0.1:5432"
```

**Discovery の場合** — 各プロジェクトに `.nameroute.toml` を置くと自動検出されます。

```toml
# config.toml
[discovery]
enabled = true
paths = ["~/workspace", "~/projects"]
```

```toml
# ~/workspace/myapp/.nameroute.toml
[[routes]]
protocol = "http"
backend = "127.0.0.1:3000"

[[routes]]
protocol = "postgres"
backend = "127.0.0.1:5432"
```

`key` を省略するとディレクトリ名がキーになります（上記なら `myapp`）。
明示的にキーを指定することも可能です：

```toml
[[routes]]
protocol = "http"
key = "api"
backend = "127.0.0.1:8000"
```

Git で管理できるため、チーム開発やプロジェクトテンプレートとの相性が良いのが特長です。

### 3. 名前でアクセスする

```bash
# HTTP — サブドメインで振り分け
curl http://myapp.localhost:8080

# PostgreSQL — データベース名で振り分け
psql -h localhost -p 15432 -d myapp

# MySQL — データベース名で振り分け
mysql -h localhost -P 13306 -D myapp
```

#### HTTPS を使う場合

HTTPS リスナーはデフォルトで有効（passthrough モード、証明書不要）です。

**Passthrough モード（デフォルト）** — TLS をそのままバックエンドに転送します。バックエンド側が TLS を処理します。設定不要でゼロコンフィグで動作します。

```bash
# バックエンドが自前で TLS を処理する場合（Next.js --experimental-https 等）
nameroute add https myapp 127.0.0.1:3443
curl https://myapp.localhost:8443
```

**Terminate モード** — name-route が TLS を終端し、バックエンドには HTTP で転送します。mkcert 等の証明書が必要です。

```bash
# 1. mkcert をインストール（初回のみ）
brew install mkcert  # macOS
# apt install mkcert  # Linux

# 2. ローカル CA をインストール
mkcert -install

# 3. ワイルドカード証明書を生成
mkcert -key-file key.pem -cert-file cert.pem "*.localhost"
```

設定ファイルに TLS セクションを追加し、ルートに `tls_mode = "terminate"` を指定します：

```toml
[tls]
cert = "cert.pem"
key = "key.pem"

[[routes]]
protocol = "https"
key = "myapp"
backend = "127.0.0.1:3000"
tls_mode = "terminate"
```

```bash
# HTTPS でアクセス（name-route が TLS を終端し、バックエンドには HTTP で転送）
curl https://myapp.localhost:8443
```

## Configuration

設定ファイルは任意です。指定しない場合はデフォルト値で動作します。

```toml
[general]
log_level = "info"         # trace, debug, info, warn, error
log_output = "stderr"      # stderr, stdout, file

[docker]
enabled = true             # false で Docker 連携を無効化
poll_interval = 3          # コンテナ検出の間隔（秒）

[listeners.http]
protocol = "http"
bind = "127.0.0.1:8080"

[listeners.postgres]
protocol = "postgres"
bind = "127.0.0.1:15432"

[listeners.mysql]
protocol = "mysql"
bind = "127.0.0.1:13306"

[listeners.smtp]
protocol = "smtp"
bind = "127.0.0.1:10025"

[http]
base_domain = "localhost"  # サブドメインの親ドメイン

[dns]
bind = "127.0.0.1:53"     # DNS サーバーのアドレス

[discovery]
enabled = true             # false で Discovery を無効化
paths = ["~/workspace"]    # 走査する親ディレクトリ
poll_interval = 3          # 走査間隔（秒）

# 静的ルート（複数定義可能）
[[routes]]
protocol = "http"
key = "myapp"
backend = "127.0.0.1:3000"
```

詳細な設定リファレンスは [Configuration Guide](../configuration.md) を参照してください。

## Docs

- [run コマンドガイド](run.md) — `nameroute run` の使い方、`$PORT` 置換、`--detect-port`、`--port-env`
- [Docker 連携ガイド](docker.md) — Docker ラベルによるルート登録、`ports:` の廃止方法
- [マイグレーションガイド](migration.md) — 既存プロジェクトからの移行手順

## License

[MIT](../../LICENSE)
