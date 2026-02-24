[English](../https.md)

# HTTPS

name-route は HTTPS リスナーをデフォルトで有効にしています。Passthrough と Terminate の2つのモードがあります。


## Passthrough mode (default)

TLS をそのままバックエンドに転送します。バックエンド側が TLS を処理するため、name-route 側の証明書は不要です。設定なしで動作します。

```bash
# バックエンドが自前で TLS を処理する場合（Next.js --experimental-https 等）
nameroute add https myapp 127.0.0.1:3443
curl https://myapp.localhost:8443
```

`nameroute run` でも利用できます:

```bash
nameroute run https myapp -- next dev --experimental-https
```


## Terminate mode

name-route が TLS を終端し、バックエンドには HTTP で転送します。バックエンドを変更せずに HTTPS でアクセスしたい場合に便利です。mkcert 等のローカル証明書が必要です。

### 1. 証明書の準備

```bash
# mkcert をインストール（初回のみ）
brew install mkcert  # macOS
# apt install mkcert  # Linux

# ローカル CA をインストール
mkcert -install

# ワイルドカード証明書を生成
mkcert -key-file key.pem -cert-file cert.pem "*.localhost"
```

### 2. 設定ファイル

TLS セクションを追加し、ルートに `tls_mode = "terminate"` を指定します:

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
sudo nameroute --config config.toml
```

### 3. アクセス

```bash
# name-route が TLS を終端し、バックエンドには HTTP で転送
curl https://myapp.localhost:8443
```
