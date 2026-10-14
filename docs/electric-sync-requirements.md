# ElectricSQL 同期連携要件（フロントエンド・バックエンド間）

フロントエンドは ElectricSQL の Shape API を購読してリアルタイム表示を行い、データの書き込みは REST API で行う構成を前提とします。このドキュメントでは、フロントエンドのリアルタイム同期と楽観的更新を成立させるために、バックエンドおよびインフラ側で満たす必要がある要件をまとめます。

フロントエンドはバックエンドの実装完了を待たず、当面はローカルの Electric に直結して開発を進めます。バックエンドの Shape プロキシが実装され次第、接続先の設定を差し替えるだけで切り替えられるよう、データ取得層を分離して実装します（詳細は「5. フロントエンドの先行開発方針」を参照）。

## 1. 読み取り（Electric Shape プロキシ）

### 通信経路と認可
クライアントライブラリには TanStack DB（`@tanstack/electric-db-collection`）を使用します。

本番および統合時の経路では、クライアントは Electric に直接アクセスせず、バックエンドの Shape プロキシエンドポイント（例: `GET /api/shapes/{resource}`）を経由します。バックエンド側で認証・認可を検証し、対象テーブル（`table`）と行フィルタ（`where`）を確定させてください。クライアントから任意の `table` や `where` を指定できないようにして認可漏れを防ぎ、Electric との通信にはサーバ側で `ELECTRIC_SECRET` を付与します。

先行開発時の経路では、ローカルの Electric（`infra/compose.yml` の `http://localhost:3000/v1/shape`、`ELECTRIC_INSECURE=true`）に直接接続します。

### 必要な Shape（現行スキーマ基準・粒度は要相談）
- `Channel`: 公開チャンネル（`is_private = false`）および自分が参加しているプライベートチャンネル
- `Message`: 指定した `channel_id` のメッセージ一覧
- `ChannelUser` / `User`: メンバー一覧・プロフィール表示用

### クエリパラメータとレスポンスヘッダの転送
Electric クライアントが送信する以下のプロトコル用・ページング（subset）用クエリパラメータを、プロキシから Electric へそのまま転送してください。

- プロトコル制御: `live`, `live_sse`, `experimental_live_sse`, `handle`, `offset`, `cursor`, `expired_handle`, `log`, `replica`, `cache-buster`
- ページング・部分取得（subset）: `subset__where`, `subset__limit`, `subset__offset`, `subset__order_by`, `subset__params`, `subset__where_expr`, `subset__order_by_expr`

また、Electric が返す `electric-handle`, `electric-offset`, `electric-schema`, `electric-cursor` 等のヘッダをクライアントへそのまま返してください（HTTP クライアントが自動展開する `content-encoding` や `content-length` は除きます）。

### `WHERE` 句のサブクエリに関する注意点
プライベートチャンネルの認可などで `WHERE` 句内のサブクエリ（例: `id IN (SELECT channel_id FROM "ChannelUser" WHERE user_id = ...)`）を使う場合、プレースホルダ（`$1` 等）をサブクエリ内に置くと Electric 側でエラーになる既知の挙動があります（`electric#3143`）。サーバ側で検証済みの ID をリテラルとして埋め込む形で回避してください。

### JOIN 不可に伴う非正規化と数値型の扱い
Electric の Shape は単一テーブル単位で配信され、テーブル間の JOIN ができません。メッセージ一覧の表示に必要な投稿者情報（`author_name`、必要に応じてアイコン URL 等）を `Message` テーブルに非正規化して持たせるか、`User` Shape を別途購読してクライアント側で結合するかを決める必要があります。

また、Electric は integer 等の数値カラムを文字列（例: `"1"`）として配信する場合があります。フロントエンド側で `Number()` による変換を行うため、API レスポンスと Shape のスキーマ定義でカラム名を揃えておけば問題ありません。

## 2. 書き込み（REST API と楽観的更新の `txid` ハンドシェイク）

### 同一トランザクションでの `txid` 返却
Electric は読み取り専用のため、作成・更新・削除はすべてバックエンドの REST API 経由で行います。フロントエンドはローカルのコレクションを楽観的に更新し、API レスポンスに含まれるトランザクション ID（`txid`）が Electric のストリームから流れてくるまで楽観状態を保持します（`collection.utils.awaitTxId(txid)`）。

そのため、すべての書き込み系エンドポイント（POST / PATCH / DELETE）は、**データの書き込みと同一の PostgreSQL トランザクション内**で以下の SQL を実行し、取得した `txid` をレスポンス JSON に含めて返してください。

```sql
SELECT pg_current_xact_id()::xid::text
```

レスポンスは以下の形式を想定しています。

```json
{
  "id": "...",
  "content": "hello",
  "txid": "12345"
}
```

書き込みと別のトランザクションで `pg_current_xact_id()` を呼ぶと `txid` が一致せず、フロントエンドの `awaitTxId` がタイムアウトします。現在バックエンドで採用している Toasty ORM（`0.11`）で同一トランザクション内の raw SQL 実行や `txid` 取得が可能か、あるいは書き込みトランザクションで `tokio-postgres` 等を併用するかの確認が必要です。

### 想定される書き込みエンドポイントとトランザクション分離
現行の `/api/users`, `/api/channels`, `/api/messages` をベースに、少なくともメッセージの送信（`POST /api/messages`）、編集（`PATCH /api/messages/{id}`）、論理削除（`DELETE /api/messages/{id}`）、およびチャンネルの作成・更新・参加（`ChannelUser` 追加）・退出の各操作で `txid` の返却が必要です。

なお、将来スレッドの返信数（`reply_count` 等）のように「子レコードの追加と同時に親レコードの集計値を更新する」処理を実装する場合、行の追加と親レコードのカウンタ更新は別トランザクションに分けてください（同一 `txid` だとクライアント側の別クエリで更新が取りこぼされる事象があるため）。

## 3. データベース・スキーマ要件

### `REPLICA IDENTITY FULL` の設定（必須）
Electric が UPDATE や DELETE の変更を正しく配信するには、対象テーブルに `REPLICA IDENTITY FULL` が設定されている必要があります。未設定の場合、INSERT は配信されますが UPDATE / DELETE が正しく同期されません。現行の `infra/db/schema.sql`（および初期マイグレーション `20261008000000_initial.sql`）には含まれていないため、以下の追加が必要です。

```sql
ALTER TABLE "User" REPLICA IDENTITY FULL;
ALTER TABLE "Channel" REPLICA IDENTITY FULL;
ALTER TABLE "Message" REPLICA IDENTITY FULL;
ALTER TABLE "ChannelUser" REPLICA IDENTITY FULL;
```

### テーブル名のケースと論理削除
現行スキーマは `"User"`, `"Channel"`, `"Message"` のように PascalCase（ダブルクォート付き識別子）で定義されています。Electric の Shape API に指定する `table` パラメータ（例: `table="Message"`）との兼ね合いを確認し、このまま PascalCase で統一するか、snake_case に変更するかを早期に確定させたいです。

`Channel.deleted_at` や `Message.deleted_at` による論理削除は、そのまま Shape で配信し、フロントエンドのクエリ側で除外する方針で対応可能です。

## 4. 認証・認可と CORS 設定

フロントエンド側は認証プロバイダを差し替えられるよう、トークン取得関数（`getToken()`）のみを持つアダプタ層を設けます。ローカル開発時は IdP なしでも動作するよう、開発用認証モード（デフォルトのローカルユーザーとして扱う、または `X-Dev-User` ヘッダでユーザーを切り替えられる仕組み）があるとフロントエンド・バックエンド双方の検証が容易になります。Shape プロキシ（読み取り）と REST API（書き込み）の両方で同じ認証ヘッダを受け付けるようにしてください。

また、ローカル開発ではフロントエンド（例: `http://localhost:3000` または `:5173`）からバックエンド（`http://localhost:8000`）へリクエストを送るため、CORS で `Authorization` 等のヘッダとオリジンを許可してください。本番・ステージング環境では `ELECTRIC_INSECURE` を無効化し、`ELECTRIC_SECRET` をバックエンドと Electric 間で共有する構成を想定しています。

## 5. フロントエンドの先行開発方針

バックエンドの Shape プロキシや認証が揃う前でも、フロントエンドは以下の手順で先行して開発を進めます。

1. `make db-up && make migrate` でローカルの PostgreSQL と Electric を起動する。
2. フロントエンドのデータ取得設定をアダプタ化し、開発初期は Electric のエンドポイント（`http://localhost:3000/v1/shape`）を直接参照する。
3. 書き込みは既存の REST API（`POST /api/channels`, `POST /api/messages` 等）またはシードデータを利用し、`txid` ハンドシェイクが実装されるまでは楽観的更新の待機をスキップ（またはモック）する。
4. バックエンド側で Shape プロキシと `txid` 返却が実装された段階で、接続先 URL を `http://localhost:8000/api/shapes/...` に切り替え、`awaitTxId` を有効化する。

## 6. バックエンド・プロダクト側と合意したい未決事項

- [ ] Workspace（サーバー）概念の有無: 現行スキーマは Workspace テーブルを持たない単一テナント構成になっているため、このまま単一テナントとして UI を設計してよいか
- [ ] 初期スコープの機能範囲: リアクション機能やスレッド機能、チャンネル内のロール管理をどのフェーズでスキーマに追加するか
- [ ] メッセージの投稿者表示: `Message` テーブルに `author_name` 等を非正規化して持たせるか、フロントエンドで `User` テーブルの Shape と結合するか
- [ ] テーブル名のケース: PascalCase（`"User"` 等）のまま進めるか、snake_case に変更するか
- [ ] Toasty ORM での `txid` 取得方法: 書き込みトランザクション内で `pg_current_xact_id()` を取得する実装方針
