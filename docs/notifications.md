# 通知の保存と管理

チャンネルとActivityPubの通知は `notification_events` を正本として保存する。`source` と `notification_type` で通知元・種類を区別し、通知ID、受信者、行為者、既読状態、日時を共有する。チャンネルは `channel_id` / `message_id`、ActivityPubは `activity_id` / `post_id` / `reaction` / `emoji` を持つ。日時はUTCの `timestamp(6)` とし、既存の連合通知のマイクロ秒精度を保持する。

通知生成と既読更新は `backend/src/notifications.rs` に集約する。既読更新は受信者を必ず条件に含め、選択ID・通知元・チャンネルで必要な範囲へ絞る。更新対象がない場合、通知行への書き込みを行わない。

- `POST /api/notifications/read` は `{ "ids": [...] }` で通知元を跨いで選択既読にする。IDの省略は入力エラー、空配列は何もしない。最大500件。他人のIDや重複IDは更新件数に含めない。
- `POST /api/notifications/read-all` は両方の通知を既読にする。`channel_id` 指定時はそのチャンネルだけを更新する。両APIは既存の `data` / `txid` 形式を使う。チャンネルShapeで観測できる変更がある場合だけ `txid` を返し、連合通知だけの更新や更新なしでは `txid: null` にする。これによりクライアントが届かないチャンネル更新を待ち続けることを防ぐ。
- 既存の単一通知・ActivityPub既読APIは互換窓口として同じ既読処理を呼ぶ。ActivityPubの窓口の全件既読はActivityPubだけを対象にする。

## チャンネルごとの通知レベル（#620）

`user_channel_notification_settings.notification_level`（`all` / `mentions` / `mute`）が設定の正本で、参加時に DEFAULT の `mentions` で行を作る。`channel_user.notification_level` は旧アプリ互換用に残すだけで、実際の設定は入れない（旧アプリが書いた値はトリガーで新しいテーブルへ移し、古い列は既定値へ戻す）。スレッド単位の設定は設けず、チャンネルごとの設定がそのチャンネルの全通知に適用する。

- `GET /api/channels/{id}/notification-settings` は `{"level":"..."}` を `Cache-Control: no-store` で返す。`PATCH` は同じ形の body を受け取り `data`/`txid` 形式で返す。チャンネルが存在し未削除で、自分自身の `channel_user` 行がある場合だけ応答し、それ以外（非メンバー・他人の設定・削除済み・不正 ID）は 404。値が不正なときは 400。同値の再送は DB を書き込まず `txid: null` にする。設定の取得に追加のポーリングや常設接続は作らない。
- 継続的な表示とトースト判定の設定は `/api/shapes/channel_notification_settings` をアカウントごとに1本購読する。実テーブルは `user_channel_notification_settings`、列は `user_id,channel_id,notification_level`、`where` は `user_id = $1` で、本人の行だけが届く。メンバー一覧の `/api/shapes/channel_users` は列を `user_id,channel_id,joined_at` に制限し、`notification_level` を一切配らない。設定が別テーブルになったため、ローリング更新中に旧バックエンドがメンバー一覧を配っても他人の設定は読めない。REST の GET/PATCH の契約は変えない。
- `mentions` は従来どおりで、`mention` / `reply` / `dm` / `reaction` を届ける。
- `all` はそのチャンネルの全通知を受け取る。通常投稿は `notification_type = 'channel_message'` として1件作る（新種別。`notification_events` に type の CHECK はない）。暗号化チャンネルでも本文を解析せず、Push は既存の暗号化プレースホルダを使う。
- `mute` には明示メンション（`@user` / `@channel` / `@here` と E2EE の `explicit_mentions.user_ids`）だけを届ける。`reply` / `reaction` / `dm` / `channel_message` は作らない。`@channel` / `@here` は `mute` でも届く例外なので、設定画面の文言もそのことが分かるようにする（文言の変更は React 側で行う）。未読カウントの扱いは変えない。
- `missed_call` はレベルの対象外で、従来どおり届ける。
- 退出（`channel_user` 行の削除）で `channel_user_notification_settings_sync` トリガーが設定行も削除する。再参加すると設定行は `mentions` で作り直され、退出前の `all` / `mute` は引き継がれない。
- APNs でも `channel_message` を通常通知（`notificationType: channel_message`）として届ける。暗号化チャンネルでは本文を解析せず既存のプレースホルダを使う。iOS アプリ側の通知レベル設定は #742 で未対応のため、iOS ではこの設定を変更できない。

`channel_user.notification_level` は旧アプリ互換列で、実際の設定は入れず常に `mentions`。旧アプリがこの列へ書いた値は `channel_user_notification_level_legacy` トリガーが `user_channel_notification_settings` へ移送し、旧列は既定値へ戻す。撤去するのは、この列を読み書きする旧アプリが本番と preview の両方で配布されなくなったことを確認した後。参加・退出時の設定行の作成・削除をバックエンドの参加・退出処理へ移し、2つのトリガー（`channel_user_notification_level_legacy` と `channel_user_notification_settings_sync`）を外してから、別PRで列を削除する。列の削除は旧アプリが動いたままだと壊れるため、移行と削除を同じPRで行わない。

1投稿につき1ユーザー1件に集約し、優先度は `mention > reply > dm > channel_message` の順（`entry().or_insert()`）。スレッド返信では、親投稿者に加えて `thread_participants` の `left_at IS NULL` な参加者へ `reply` を出す。公開チャンネルは非メンバーも対象にでき、非公開は `channel_user` 所属を確認する。本人・bot・`is_deactivated`・退出済みの参加者は除外し、公開チャンネルから外されたゲストにも届けない。Home 経由の投稿は、事前に読み取った `messages.parent_id`（正規化済みのルート。`collecting` 中は直接の返信先）を参加者の検索キーに使う。探索上限と実行計画の方針は [SQLの設計と性能基準](query-design.md) の「通知レベルとスレッド参加者への返信通知」を参照。

運用上限: 1返信の通知先候補数 `T`（`all` 設定者 + 生存スレッド参加者 + メンション等の合計）は **10,000 以下**に保つ。返信はルート行を `FOR NO KEY UPDATE` で通知 INSERT の完了まで保持するため、`T` に比例して同じルートへの並列返信・参加・退出が待つ（2 セッションだけの観測では、ロック待ちが T=10,000 で約2〜3秒、T=150,000 で約34〜35秒。別途一括 INSERT を計測した 42.2〜42.5 秒は INSERT の時間で、ロック待ちではない）。さらに同じルートへ k 件が重なると待ちは約 (k-1) × 1件の処理時間で累積する。8同時・T=10,000 の実測では、**SQL 処理全体**（ロック待ち＋通知 INSERT＋COMMIT、psql の往復を含む）が p50 15.9秒 / p95 64.8秒 / 最大 79.0秒で、**79.0秒はロック待ちだけの値ではない**。返信 API の「リクエスト開始→レスポンス受信」はタイムアウト導入前の 8並列30回（240サンプル、返信者8人を `mute` にして1返信あたり10,000行を SQL で確認）で p50 11.7秒 / p95 22.0秒 / 最大 71.9秒。**`lock_timeout` 使用時（`statement_timeout` 変更前）**では同じ 8並列30回（240件）で成功 240件 / 503 0件、成功の p50 9.5秒 / p95 19.1秒 / 最大 19.5秒。503 は1件も出ず応答時間は未算出（`api_total_service_unavailable` は `null`）。240件すべてで9ステージと1返信あたり10,000行を確認し、503時に投稿・通知0件を検証するコードはそのまま使った（対象件なし）。この実行では240件すべてが成功し、`lock_timeout` による503は観測されなかった（個々のロック取得待ちは未計測で、5秒未満だったとは判断できない）。**`SET LOCAL statement_timeout = '5s'`（取得後に `DEFAULT` へ戻す）変更後**では同じ 8並列30回（240件）で成功 87件 / 503 153件、成功の p50 4.7秒 / p95 7.3秒 / 最大 22.3秒、503 の p50 5.0秒 / p95 5.1秒 / 最大 5.1秒。成功87件すべてで9ステージと1返信あたり10,000行を確認し、503の153件は `code=THREAD_BUSY` と `retry_after=5` の本文を返し、投稿・通知0件を SQL で確認した。本番の同時返信数や `all` 設定者の分布での受容上限を示すものではない。in-process ルーター経由のため HTTP ネットワークは除外。内訳は本番コードの `notify_timing` イベント（`tracing::debug!`、計測時のみ有効）をテスト側でメッセージ ID ごとに収集して計測し、`lock_timeout` 使用時はルーター自身の接続取得 p50 0.14ms、ルート行ロック区間 p50 7.1秒 / 最大 17.0秒（2回のロック取得とゲート判定を含む）、メンバー取得 p50 38ms と設定取得 p50 36ms、通知 INSERT p50 2.2秒 / 最大 2.5秒、COMMIT p50 4ms。`statement_timeout` 変更後は接続取得 p50 0.137ms、ルート行ロック区間 p50 2.3秒 / 成功87件の観測最大 4.9秒（5秒の上限はSQL文ごと。区間には2回のロック取得とゲート判定を含むため、接続占有時間を過小評価しない）、メンバー取得 p50 38.7ms と設定取得 p50 35.8ms、通知 INSERT p50 2.2秒 / 最大 19.8秒（取得後に `DEFAULT` へ戻すため5秒を超えうる）、COMMIT p50 3.8ms。タイムアウト導入前の旧値（ルート行ロック最大 69.4秒 / 通知 INSERT 最大 27.6秒）は `docs/query-design.md` に別途記録している。各 SQL の値は往復を含む実行時間で、Postgres の `Planning Time` だけは toasty ドライバに計測点が無いため分離していない。上限超過時は配信を止めず、`notifications` ターゲットの警告ログ（`notification targets exceed the operational limit`、`targets` と `limit` フィールド付き）を出す。境界は候補 10,000 件で警告なし、10,001 件で警告（`backend/src/notifications.rs` の単体テストで検証）。運用ではこの警告をログ監視して `all` 設定者やスレッド参加者を絞る。返信のルート行ロック待ちは `statement_timeout`（取得後に `DEFAULT` へ戻す）で制限し、57014／55P03 は 503＋`Retry-After: 5` に変換する。503 では投稿も通知も残らないため、クライアントは同じ `id` で再送できる（`docs/threads.md` 参照）。実測値と上限の根拠、レイヤ別の数値、タイムアウト導入前後の内訳は [SQLの設計と性能基準](query-design.md) の「返信のルート行ロック保持時間と並列返信の待ち」を参照。100ms は参考値で、時間だけで合否を決めない。

設定を専用テーブルへ分けるマイグレーション（`20261208000002`）は `channel_user` を `SHARE ROW EXCLUSIVE ... NOWAIT` でロックする。取得できなければ即時に失敗して移行ファイルは未適用のまま戻り、再デプロイで再試行する（migrate Job の `backoffLimit: 3` を使い切るとデプロイが失敗する）。取得後はバックフィル完了まで参加・退出・旧列の更新が待つ。

`channel_notifications` と `federated_notifications` は共通テーブルの更新可能なビューで、別の保存先ではない。チャンネルの既存DTOと連合通知のアクター・投稿表示用DTOは各ビューから取得する。Electricの通知Shapeも共通テーブルからチャンネル通知を配信し、従来の8列だけを返す。チャンネル・メッセージの削除、非公開チャンネルの所属確認、ユーザー別フィルターは引き続き適用する。React・Composeのクライアント側スキーマやUIを変更せず利用できる。

## 不在着信（`missed_call`）

通話の呼び出し（[音声通話と呼び出し](calls.md)）に応答がなかったときの通知で、`notification_type = 'missed_call'` の `channel` 通知として残す。メッセージを持たない唯一の種類なので、`message_id` は NULL、CHECK 制約（`notification_events_source_shape`）は `missed_call` のときだけそれを許す。`message_deleted` は `false` にして通知 Shape に出し、旧 `notifications` テーブルへは写さない。通知 API と Shape の `message_id` は `null` になりうる。

## 移行と互換性

`20261102140000_unify_notification_storage.sql` は既存の2テーブルをロックしてから、ID・通知本文用メタデータ・既読状態・日時を共通テーブルへ移す。チャンネルの日時はUTCとして扱い、ActivityPubの `timestamptz` はUTCへ変換する。

ロック・バックフィル・同期トリガーの追加は単一の `DO` ブロックで実行し、Atlasの検証時に外側のトランザクションがない場合も原子的に適用する。

旧 `notifications` / `apub_notifications` はローリングデプロイ中の旧アプリとの互換用に残す。両方向の行トリガーで作成・更新・削除を同じトランザクション内に反映する。内容が同じ行のUPSERTは更新しないため、相互呼び出しは収束する。旧テーブルの外部キーによる削除も共通テーブルへ反映し、連合通知の更新は既存のHomeライブ配信を継続する。旧テーブルの削除は、新実装の本番反映を確認した後の別PRで扱う。

`20261208000001_apub_quote_grants.sql` は main の `20261205000002_apub_quote_grants.sql` を改番したもの。このブランチの preview DB は既に 20261208 系のマイグレーションを適用済みで、後から `20261205000002` を追加すると Atlas が out-of-order として適用を拒否するため、ファイル名を `20261208000001` に維持する。BASE 側のマイグレーションが適用済みの DB でも冪等に再実行できるよう、テーブル・インデックスは `IF NOT EXISTS`、トリガーは `DROP ... IF EXISTS` の後に作り直す形にしている。

検証先は `backend/tests/notification_storage_test.rs`。通知元を跨いだ選択／全件既読、所有者・チャンネル境界、空選択・重複ID・上限、旧／新書き込みの同期、メタデータ、削除と外部キーの連鎖、Electricの配信テーブル・列・権限フィルターを確認する。既存のActivityPub・通知・通知コンテキストのAPIテストも共通保存先で実行する。

今回の検証: 通知・通知コンテキスト・共通DBのAPI統合テスト9件、ActivityPub APIテスト18件、Shape単体テスト7件が成功。空DBへの全マイグレーション適用、既存2種類の通知を含むDBの移行、ID・既読状態・日時・絵文字メタデータの保持、スキーマ直接適用とマイグレーション適用での新テーブル・ビュー・関数・トリガー・制約・インデックスの一致、Atlasチェックサム検証も確認した。Atlasの差分生成は関数のログイン要求で停止するため、新規SQLを適用・定義比較で検証した。

## Reactの購読と取得

シェル配下の `NotificationsProvider` で通知バッジと画面の一覧・ページング・既読状態を共有する。Home用WebSocketも `HomeLiveProvider` で他のHome・ActivityPub画面と共有する。取得中の更新は前のリクエストを中断せず集約し、完了後に再取得する。通信経路とポーリングの停止・集約方針は [リクエストと接続の設計](request-design.md) を参照。

AndroidはSessionの `ActivityPubNotificationsState` と共有 `HomeLive` を使い、取得中の更新をキャンセルせず予約する。追加ページは進行中の先頭更新を待ち、画面・バッジで同じ保持範囲を共有する。
