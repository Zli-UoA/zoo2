# 通知の保存と管理

チャンネルとActivityPubの通知は `notification_events` を正本として保存する。`source` と `notification_type` で通知元・種類を区別し、通知ID、受信者、行為者、既読状態、日時を共有する。チャンネルは `channel_id` / `message_id`、ActivityPubは `activity_id` / `post_id` / `reaction` / `emoji` を持つ。日時はUTCの `timestamp(6)` とし、既存の連合通知のマイクロ秒精度を保持する。

通知生成と既読更新は `backend/src/notifications.rs` に集約する。既読更新は受信者を必ず条件に含め、選択ID・通知元・チャンネルで必要な範囲へ絞る。更新対象がない場合、通知行への書き込みを行わない。

- `POST /api/notifications/read` は `{ "ids": [...] }` で通知元を跨いで選択既読にする。IDの省略は入力エラー、空配列は何もしない。最大500件。他人のIDや重複IDは更新件数に含めない。
- `POST /api/notifications/read-all` は両方の通知を既読にする。`channel_id` 指定時はそのチャンネルだけを更新する。両APIは既存の `data` / `txid` 形式を使う。チャンネルShapeで観測できる変更がある場合だけ `txid` を返し、連合通知だけの更新や更新なしでは `txid: null` にする。これによりクライアントが届かないチャンネル更新を待ち続けることを防ぐ。
- 既存の単一通知・ActivityPub既読APIは互換窓口として同じ既読処理を呼ぶ。ActivityPubの窓口の全件既読はActivityPubだけを対象にする。

`channel_notifications` と `federated_notifications` は共通テーブルの更新可能なビューで、別の保存先ではない。チャンネルの既存DTOと連合通知のアクター・投稿表示用DTOは各ビューから取得する。Electricの通知Shapeも共通テーブルからチャンネル通知を配信し、従来の8列だけを返す。チャンネル・メッセージの削除、非公開チャンネルの所属確認、ユーザー別フィルターは引き続き適用する。React・Composeのクライアント側スキーマやUIを変更せず利用できる。

## 不在着信（`missed_call`）

通話の呼び出し（[音声通話と呼び出し](calls.md)）に応答がなかったときの通知で、`notification_type = 'missed_call'` の `channel` 通知として残す。メッセージを持たない唯一の種類なので、`message_id` は NULL、CHECK 制約（`notification_events_source_shape`）は `missed_call` のときだけそれを許す。`message_deleted` は `false` にして通知 Shape に出し、旧 `notifications` テーブルへは写さない。通知 API と Shape の `message_id` は `null` になりうる。

## 移行と互換性

`20261102140000_unify_notification_storage.sql` は既存の2テーブルをロックしてから、ID・通知本文用メタデータ・既読状態・日時を共通テーブルへ移す。チャンネルの日時はUTCとして扱い、ActivityPubの `timestamptz` はUTCへ変換する。

ロック・バックフィル・同期トリガーの追加は単一の `DO` ブロックで実行し、Atlasの検証時に外側のトランザクションがない場合も原子的に適用する。

旧 `notifications` / `apub_notifications` はローリングデプロイ中の旧アプリとの互換用に残す。両方向の行トリガーで作成・更新・削除を同じトランザクション内に反映する。内容が同じ行のUPSERTは更新しないため、相互呼び出しは収束する。旧テーブルの外部キーによる削除も共通テーブルへ反映し、連合通知の更新は既存のHomeライブ配信を継続する。旧テーブルの削除は、新実装の本番反映を確認した後の別PRで扱う。

検証先は `backend/tests/notification_storage_test.rs`。通知元を跨いだ選択／全件既読、所有者・チャンネル境界、空選択・重複ID・上限、旧／新書き込みの同期、メタデータ、削除と外部キーの連鎖、Electricの配信テーブル・列・権限フィルターを確認する。既存のActivityPub・通知・通知コンテキストのAPIテストも共通保存先で実行する。

今回の検証: 通知・通知コンテキスト・共通DBのAPI統合テスト9件、ActivityPub APIテスト18件、Shape単体テスト7件が成功。空DBへの全マイグレーション適用、既存2種類の通知を含むDBの移行、ID・既読状態・日時・絵文字メタデータの保持、スキーマ直接適用とマイグレーション適用での新テーブル・ビュー・関数・トリガー・制約・インデックスの一致、Atlasチェックサム検証も確認した。Atlasの差分生成は関数のログイン要求で停止するため、新規SQLを適用・定義比較で検証した。

## Reactの購読と取得

シェル配下の `NotificationsProvider` で通知バッジと画面の一覧・ページング・既読状態を共有する。Home用WebSocketも `HomeLiveProvider` で他のHome・ActivityPub画面と共有する。取得中の更新は前のリクエストを中断せず集約し、完了後に再取得する。通信経路とポーリングの停止・集約方針は [リクエストと接続の設計](request-design.md) を参照。

AndroidはSessionの `ActivityPubNotificationsState` と共有 `HomeLive` を使い、取得中の更新をキャンセルせず予約する。追加ページは進行中の先頭更新を待ち、画面・バッジで同じ保持範囲を共有する。
