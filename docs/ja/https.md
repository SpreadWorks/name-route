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

> **Tip:** マルチレベルサブドメイン（例: `api.myapp.localhost`）を使う場合は証明書に追加してください:
> ```bash
> mkcert -key-file key.pem -cert-file cert.pem "*.localhost" "*.myapp.localhost"
> ```

### 2. 設定ファイル

設定ファイルに TLS セクションを追加します:

```toml
[tls]
cert = "cert.pem"
key = "key.pem"
```

### 3. `nameroute run` で使う

`--tls-mode terminate` を指定すると、name-route が TLS を処理します。バックエンドは HTTP のまま動作するため、`--experimental-https` などのフラグは不要です。

```bash
nameroute run https myapp --tls-mode terminate -- next dev --port '$PORT'
```

```
curl ──tls──▶ nameroute:8443 ──http──▶ next:$PORT (HTTP)
```

#### package.json の例

```json
{
  "scripts": {
    "dev": "nameroute run https myapp --tls-mode terminate -- next dev"
  }
}
```

### 4. 静的ルートで使う

ルートに `tls_mode = "terminate"` を指定します:

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
curl https://myapp.localhost:8443
```


## Passthrough と Terminate の比較

| | Passthrough（デフォルト） | Terminate |
|---|---|---|
| TLS の処理 | バックエンド | name-route |
| 証明書 | バックエンドが管理 | name-route の設定（`[tls]`） |
| バックエンドのプロトコル | HTTPS | HTTP |
| ユースケース | バックエンドが TLS を提供する場合 | バックエンドを変更せずに HTTPS 化 |
| `nameroute run` | `nameroute run https myapp -- next dev --experimental-https` | `nameroute run https myapp --tls-mode terminate -- next dev` |

> **Note:** Passthrough モードでは、証明書のドメインがルーティングキー（例: `myapp.localhost`）と一致する必要があります。Next.js の `--experimental-https` が生成する証明書は通常 `localhost` のみが対象のため、ドメイン不一致の警告が出ます。これを避けるには terminate モードを使ってください。
