# ElectricSQL 同期連携要件（フロントエンド・バックエンド間）

フロントエンドは ElectricSQL の Shape API を購読してリアルタイム表示を行い、データの書き込みは REST API で行う構成を前提とします。このドキュメントでは、フロントエンドのリアルタイム同期と楽観的更新を成立させるために、バックエンドおよびインフラ側で満たす必要がある要件をまとめます。

フロントエンドはバックエンドの実装完了を待たず、当面はローカルの Electric に直結して開発を進めます。バックエンドの Shape プロキシが実装され次第、接続先の設定を差し替えるだけで切り替えられるよう、データ取得層を分離して実装します（詳細は「5. フロントエンドの先行開発方針」を参照）。

## 1. 読み取り（Electric Shape プロキシ）

### 通信経路と認可
クライアントライブラリには TanStack DB（`@tanstack/electric-db-collection`）を使用します。

本番および統合時の経路では、クライアントは Electric に直接アクセスせず、バックエンドの Shape プロキシエンドポイント（例: `GET /api/shapes/{resource}`）を経由します。バックエンド側で認証・認可を検証し、対象テーブル（`table`）と行フィルタ（`where`）を確定させてください。クライアントから任意の `table` や `where` を指定できないようにして認可漏れを防ぎ、Electric との通信にはサーバ側で `ELECTRIC_SECRET` を付与します。

先行開発時の経路では、ローカルの Electric（`infra/db/compose.yml` の `http://localhost:3000/v1/shape`、`ELECTRIC_INSECURE=true`）に直接接続します。

### 必要な Shape（現行スキーマ基準・粒度は要相談）
- `Channel`: 公開チャンネルは全認証ユーザーが閲覧できます（後述の単一テナント構成）。プライベートチャンネルは `ChannelUser` に登録されたユーザーのみ閲覧できます。
- `Message`: 指定した `channel_id` のメッセージ一覧。
- `User`: 全ユーザー。メッセージの投稿者表示をクライアント側で結合するために購読します（非正規化はしません）。
- `ChannelUser`: メンバー一覧・参加状態の表示用。

### クエリパラメータとレスポンスヘッダの転送
Electric クライアントが送信する以下のプロトコル用・ページング（subset）用クエリパラメータを、プロキシから Electric へそのまま転送してください。

- プロトコル制御: `live`, `live_sse`, `experimental_live_sse`, `handle`, `offset`, `cursor`, `expired_handle`, `log`, `replica`, `cache-buster`
- ページング・部分取得（subset）: `subset__where`, `subset__limit`, `subset__offset`, `subset__order_by`, `subset__params`, `subset__where_expr`, `subset__order_by_expr`

また、Electric が返す `electric-handle`, `electric-offset`, `electric-schema`, `electric-cursor` 等のヘッダをクライアントへそのまま返してください（HTTP クライアントが自動展開する `content-encoding` や `content-length` は除きます）。

### SSE とストリーミング転送
`live=true` や `live_sse=true` の応答は Server-Sent Events です。プロキシ側でレスポンスをバッファリングせず、チャンクのままクライアントへ透過的に流してください。応答全体を一度に読み込む実装（`bytes()` 等でまとめて取得する方式）は、SSE では応答が返らず停止するため使えません。

### `WHERE` 句のサブクエリに関する注意点
プライベートチャンネルの認可などで `WHERE` 句内のサブクエリ（例: `id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)`）を使います。

以前は、プレースホルダ（`$1` 等）をサブクエリ内に置くと Electric 側でエラーになる既知の挙動（`electric#3143`）があったため、サーバ側で検証済みの値をリテラルとして埋め込む回避策が必要とされていました。Phase 5（#17）で Electric 1.8.1 を `ELECTRIC_FEATURE_FLAGS=allow_subqueries` 付きで動かして再確認したところ、`params[1]` 付きのサブクエリがそのまま通り、依存 Shape によりチャンネル参加/退出が move-in / move-out として配信されることを実測しました（`infra/db/compose.yml` / `infra/compose.stg.yaml` で `allow_subqueries` を設定済み）。そのため現状はリテラルを埋め込む回避策を入れておらず、バックエンドの `routes/shapes.rs` もプレースホルダのままで実装しています。`allow_subqueries` を無効化する構成へ変える場合は、この前提を再確認してください。

なお、現行スキーマの主キーは `text` 型であり、UUID ではありません。主キーの型（`text` のままか UUID にするか）は「7. 要検討事項」とします。`allow_subqueries` が使える前提では値はプレースホルダで渡すため、リテラル埋め込み時のインジェクション検証は不要です。

### JOIN 不可に伴うクライアント側結合
Electric の Shape は単一テーブル単位で配信され、テーブル間の JOIN ができません。投稿者名などの関連情報は、`Message` テーブルへ非正規化せず、`User` Shape を別途購読して TanStack DB の live query でクライアント側結合する方針とします。

非正規化しない理由は、ユーザーが表示名やアイコンを変更した際に過去ログを一括更新する必要がなく、クライアント側結合なら変更が即座に全メッセージへ反映されるためです。

このとき、`User` Shape のスコープは「表示しうる全メッセージの投稿者」を必ず含むようにしてください。単一テナント構成では全ユーザーを同期対象にするため、この条件は満たされます。

### 数値型の扱い
Electric は integer 等の数値カラムを文字列（例: `"1"`）として配信する場合があります。フロントエンド側で `Number()` による変換を行うため、API レスポンスと Shape のスキーマ定義でカラム名を揃えておけば問題ありません。

## 2. 書き込み（REST API と楽観的更新の `txid` ハンドシェイク）

### 同一トランザクションでの `txid` 返却
Electric は読み取り専用のため、作成・更新・削除はすべてバックエンドの REST API 経由で行います。フロントエンドはローカルのコレクションを楽観的に更新し、API レスポンスに含まれるトランザクション ID（`txid`）が Electric のストリームから流れてくるまで楽観状態を保持します（`collection.utils.awaitTxId(txid)`）。

そのため、すべての書き込み系エンドポイント（POST / PATCH / DELETE）は、**データの書き込みと同一の PostgreSQL トランザクション内**で以下の SQL を実行し、取得した `txid` をレスポンスに含めて返してください。

```sql
SELECT pg_current_xact_id()::xid::text
```

書き込みと別のトランザクションで `pg_current_xact_id()` を呼ぶと `txid` が一致せず、フロントエンドの `awaitTxId` がタイムアウトします。

現行の Toasty ORM（`0.11`）で、書き込みトランザクション内からこの SQL を実行できるかを確認する必要があります。実行できる場合は Toasty のトランザクション経由で取得します。実行できない場合は、書き込み本体を `tokio-postgres` の明示的なコネクションに寄せ、1 文（例: `INSERT ... RETURNING ..., pg_current_xact_id()::xid::text AS txid`）で書き込みと `txid` 取得を完結させます。ただしこの場合は書き込み経路が Toasty と `tokio-postgres` の二系統になり、Toasty の役割が読み取り中心に縮むことを許容するかの判断が必要です（「7. 要検討事項」）。

### 想定される書き込みエンドポイントとトランザクション分離
現行の `/api/users`, `/api/channels`, `/api/messages` をベースに、少なくともメッセージの送信（`POST /api/messages`）、編集（`PATCH /api/messages/{id}`）、論理削除（`DELETE /api/messages/{id}`）、およびチャンネルの作成・更新・参加（`ChannelUser` 追加）・退出の各操作で `txid` の返却が必要です。

なお、将来スレッドの返信数（`reply_count` 等）のように「子レコードの追加と同時に親レコードの集計値を更新する」処理を実装する場合、行の追加と親レコードのカウンタ更新は別トランザクションに分けてください（同一 `txid` だとクライアント側の別クエリで更新が取りこぼされる事象があるため）。

### `txid` レスポンスの形式
各エンドポイントのレスポンスは、共通エンベロープ（例: `{ "data": ..., "txid": "..." }`）に統一することを推します。フロントエンドはアダプタで `data` と `txid` を取り出すため、行をそのまま返す形式（`{ ...row, "txid": "..." }`）でも実装コストは変わりませんが、一覧取得や削除など形の異なるレスポンスでも扱いを揃えられる点でエンベロープが扱いやすいです。最終的な形式は実装開始時に確定させてください。

## 3. データベース・スキーマ要件

### `REPLICA IDENTITY FULL` の設定（必須）
Electric が UPDATE や DELETE の変更を正しく配信するには、対象テーブルに `REPLICA IDENTITY FULL` が設定されている必要があります。未設定の場合、INSERT は配信されますが UPDATE / DELETE が正しく同期されません。現行の `infra/db/schema.sql`（および初期マイグレーション `20261008000000_initial.sql`）には含まれていないため、以下の追加が必要です。

```sql
ALTER TABLE users REPLICA IDENTITY FULL;
ALTER TABLE channels REPLICA IDENTITY FULL;
ALTER TABLE messages REPLICA IDENTITY FULL;
ALTER TABLE channel_user REPLICA IDENTITY FULL;
```

Atlas は宣言的スキーマでの `REPLICA IDENTITY` の差分検知を標準でサポートしていない場合があるため、`infra/db/migrations/` に手動のマイグレーションファイルを作成して適用する運用とします。

### テーブル名は `snake_case` に統一
現行スキーマは `"User"`, `"Channel"`, `"Message"` のように PascalCase（ダブルクォート付き識別子）で定義されていますが、`snake_case`（`users`, `channels`, `messages`, `channel_user`, `role`, `user_role`）へ統一します。ダブルクォート付きの識別子は、PostgreSQL の自動小文字化の影響で、Electric の `table` パラメータ指定・Atlas の差分検知・生 SQL のいずれでもクォート漏れを起こしやすいためです。本番運用前の現時点で統一します。

### 主キーの型（要検討）
現行の主キーは `text` です。サブクエリへリテラルを埋め込む際の安全性検証や、API と Shape 間の型の一貫性を考えると、UUID（`uuid` 型）に寄せる選択肢もあります。どちらに統一するかは「7. 要検討事項」とします。

### 論理削除の扱い
`Channel.deleted_at` や `Message.deleted_at` による論理削除は、そのまま Shape で配信し、フロントエンドのクエリ側で除外する方針で対応可能です。

## 4. 認証・認可と CORS 設定

フロントエンド側は認証プロバイダを差し替えられるよう、トークン取得関数（`getToken()`）のみを持つアダプタ層を設けます。ローカル開発時は IdP なしでも動作するよう、開発用認証モード（デフォルトのローカルユーザーとして扱う、または `X-Dev-User` ヘッダでユーザーを切り替えられる仕組み）があるとフロントエンド・バックエンド双方の検証が容易になります。Shape プロキシ（読み取り）と REST API（書き込み）の両方で同じ認証ヘッダを受け付けるようにしてください。

また、ローカル開発ではフロントエンド（例: `http://localhost:3000` または `:5173`）からバックエンド（`http://localhost:8000`）へリクエストを送るため、CORS で `Authorization` 等のヘッダとオリジンを許可してください。本番・ステージング環境では `ELECTRIC_INSECURE` を無効化し、`ELECTRIC_SECRET` をバックエンドと Electric 間で共有する構成を想定しています。

## 5. フロントエンドの先行開発方針

バックエンドの Shape プロキシや認証が揃う前でも、フロントエンドは以下の手順で先行して開発を進めます。

1. `make db-up && make migrate` でローカルの PostgreSQL と Electric を起動する。
2. フロントエンドのデータ取得設定をアダプタ化し、開発初期は Electric のエンドポイント（`http://localhost:3000/v1/shape`）を直接参照する。
3. `Message` と `User` の Shape を購読し、TanStack DB の live query でクライアント側結合して投稿者を表示する。
4. 書き込みは既存の REST API（`POST /api/channels`, `POST /api/messages` 等）またはシードデータを利用し、`txid` ハンドシェイクが実装されるまでは楽観的更新の待機をスキップ（またはモック）する。
5. バックエンド側で Shape プロキシと `txid` 返却が実装された段階で、接続先 URL を `http://localhost:8000/api/shapes/...` に切り替え、`awaitTxId` を有効化する。

## 6. 決定事項

- 単一テナント（Workspace なし）で進める。公開チャンネルは全認証ユーザーが閲覧でき、プライベートチャンネルは `ChannelUser` に登録されたユーザーのみ閲覧できる。
- 初期スコープはチャンネルの作成・一覧・参加・退出と、メッセージの送受信・編集・論理削除とする。スレッドは集計カラムの `txid` 分離問題があるため Phase 2 とする。リアクションは集計を持たず他の書き込みと同じ `txid` ハンドシェイクで扱えるため、必要なら前倒しできる。
- 投稿者表示は非正規化せず、`User` Shape を購読してクライアント側で結合する。
- テーブル名は `snake_case` に統一する。
- `REPLICA IDENTITY FULL` は Atlas の手動マイグレーションで追加する。

## 7. 要検討事項

- [ ] 書き込み処理の `txid` 取得方法: Toasty のトランザクション API で生 SQL を実行できるかを確認する。実行できない場合、書き込み本体を `tokio-postgres` に寄せて Toasty を読み取り中心にする方針を許容するか。
- [ ] 主キーの型: `text` のまま進めるか、UUID に変更するか。
- [ ] `txid` レスポンスの形式: 共通エンベロープ（`{ data, txid }`）か、行に `txid` を足す形式か。
