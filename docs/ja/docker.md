# Docker 連携ガイド

name-route は Docker コンテナのラベルからルーティング情報を自動検出します。
これにより、`docker-compose.yml` の `ports:` セクションを廃止し、
ポート番号の管理から完全に解放されます。

## 基本的なラベル設定

```yaml
services:
  web:
    image: nginx
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":80}]'
```

name-route はコンテナの IP アドレスとラベルで指定されたポートを組み合わせて、
自動的にルートを登録します。

## ports: を廃止する

従来の `docker-compose.yml`:

```yaml
services:
  web:
    image: nginx
    ports:
      - "3000:80"       # ← ポート番号を覚える必要がある
  db:
    image: postgres
    ports:
      - "5432:5432"     # ← 他のプロジェクトと衝突する可能性
```

name-route を使った `docker-compose.yml`:

```yaml
services:
  web:
    image: nginx
    # ports: は不要！
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":80}]'
  db:
    image: postgres
    # ports: は不要！
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

**メリット:**
- ポート番号の衝突が起こらない（name-route が名前で振り分けるため）
- 複数プロジェクトを同時に起動できる
- `docker-compose.yml` がシンプルになる

## ラベルの書式

ラベル値は JSON 配列で、複数のルートを一度に定義できます。

```json
[
  {"protocol": "http", "key": "myapp", "port": 80},
  {"protocol": "postgres", "key": "myapp"}
]
```

| フィールド | 必須 | 説明 |
|-----------|------|------|
| `protocol` | Yes | `http`, `https`, `postgres`, `mysql`, `smtp` のいずれか |
| `key` | No | ルーティングキー。省略時はコンテナ名 |
| `port` | No | コンテナ内のポート。省略時はプロトコルのデフォルトポート |

デフォルトポート:
- HTTP: 80
- HTTPS: 443
- PostgreSQL: 5432
- MySQL: 3306
- SMTP: 25

## docker run でのラベル指定

`docker-compose.yml` を使わない場合でも、`docker run` の `--label` オプションでラベルを指定できます。

```bash
# 単一ルート
docker run --label 'name-route=[{"protocol":"http","key":"myapp","port":80}]' nginx

# 複数ルート
docker run --label 'name-route=[{"protocol":"http","key":"myapp","port":3000},{"protocol":"postgres","key":"myapp"}]' myapp
```

`docker compose run` でも同様です:

```bash
docker compose run --label 'name-route=[{"protocol":"http","key":"myapp","port":80}]' web
```

> **Note:** `docker compose run` で指定したラベルは、`docker-compose.yml` の `labels:` より優先されます。
> 一時的に別のキーでルートを登録したい場合などに便利です。

## 複数ルートの例

一つのコンテナで複数のプロトコルを公開する場合:

```yaml
services:
  app:
    image: myapp
    labels:
      name-route: |
        [
          {"protocol":"http","key":"myapp","port":3000},
          {"protocol":"postgres","key":"myapp","port":5432}
        ]
```

## コンテナの起動・停止への追従

name-route は定期的に Docker をポーリング（デフォルト 3 秒間隔）し、
コンテナの起動・停止を検出します。

- コンテナが起動 → ルートが自動登録
- コンテナが停止 → ルートが自動削除

ポーリング間隔は設定で変更できます:

```toml
[docker]
enabled = true
poll_interval = 3  # 秒
```

## Docker を使わないサービスとの共存

Docker コンテナと、ホスト側で動かす開発サーバーを組み合わせることができます。

```yaml
# docker-compose.yml — DB だけ Docker
services:
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

```bash
# アプリはホスト側で起動
nameroute run http myapp -- next dev
```

```bash
# 両方とも名前でアクセス
curl http://myapp.localhost:8080       # → ホスト側の Next.js
psql -h localhost -p 15432 -d myapp    # → Docker の PostgreSQL
```
