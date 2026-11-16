# スレッド参加と段階移行

スレッド参加はツリーへの表示設定であり、閲覧権限ではない。公開・非公開チャンネルの既存の閲覧権限を使い、未参加者も親メッセージの返信リンクから内容を開ける。チャンネル参加者はヘッダーの「参加」で投稿せず参加できる。チャンネル未参加者は先にチャンネルへ参加する。

Openの参加スレッドをツリーへ表示する。未参加のOpenは閲覧中だけ表示する。Archivedは閲覧中も表示せず、履歴・検索UIを設けない。ヘッダーの「退出」は参加を解除し、閉じる操作とは独立する。再Openは退出を取り消さない。本人の新規投稿では再参加するが、同じ投稿IDの再送や過去履歴の取り込みでは再参加しない。親投稿者と初返信者は自動参加する。

スレッドのペインのヘッダーに、参加中の人数と一覧（先頭から最大50人）を出す。`GET /api/threads/{id}` の応答の `participant_count` と `participants` を使い、参加状態と同じ取得・更新周期で更新する。退出した人と無効化したユーザーは含めない。閲覧権限は付与しない。

アーカイブでは参加行を消さない。手動再Openと投稿による再Openの両方で、退出していない参加者のツリーへ戻る。再Open操作自体は操作者を新しく参加させない。Archived中は退出できるが、新規参加は再Open後に行う。

## 展開手順

本変更はデプロイだけで既存データを一括更新しない。以下は対象環境の運用者が行う。通常のバックアップが取得できていること、旧バックエンド・ワーカーが停止したことを確認してから `start` を実行する。

1. 通常のDBマイグレーションを適用する。新しいテーブル・列・トリガーが入り、`thread_rollout.phase=collecting` となる。旧アプリの投稿も参加情報を記録するが、既存の親IDは変えず、一覧の参加フィルターも適用しない。
2. この変更を含むバックエンド・クライアントを展開する。移行中は従来の一覧範囲を保つため、参加フィルターが効くのは `finish` 後となる。
3. 対象DBの `DATABASE_URL` を設定し、`scripts/migrate-threads.sh status` と事前調査を行う。`start` は循環・親欠損・暗号化された入れ子を検出すると進まない。祖先探索は親を持つ行だけに限定する。チャンネルをまたぐ親、削除済み祖先、深さ・件数も調査し、不整合があれば修復方針を決める。
4. `scripts/migrate-threads.sh start` で `normalizing` に進める。以後、旧クライアントが子IDを親に指定しても、DBで平文の新規投稿の親を最上位へ正規化する。暗号化返信は暗号文が親IDを認証するため、自動的に親を変更せず、最上位IDを使って再暗号化するよう400を返す。最上位親が削除済みなら404を返す。
5. `scripts/migrate-threads.sh batch` を低い頻度で繰り返す。既定で1スレッドとHome100件を別々に確定する。`status` と投稿レイテンシを確認し、負荷に余裕がある場合だけ `batch 10` などへ増やす。自動の無制限ループは設けない。
6. `scripts/migrate-threads.sh finish` が残件・入れ子なしを検証し、`ready` にする。ここで一覧の参加フィルターを有効にする。A/Bの会話をCが親から開いて参加・退出できること、退出後の再Open、Archivedの非表示、旧URLを確認する。

ロック待ちは2秒、1文は30秒で打ち切る。バッチ中は対象スレッドへの書き込みが待つ可能性があるが、全メッセージをロックする一括移行は行わない。実際の所要時間は返信数・索引・同時負荷に依存する。巨大な1スレッドが30秒を超えた場合はそのバッチが戻るため、負荷・対象を調査してから別途対応する。上限を自動で延長しない。調査したうえで運用者が明示的に延長する場合に限り、`THREAD_MIGRATION_STATEMENT_TIMEOUT=60s scripts/migrate-threads.sh start` のように指定できる。`ms`・`s`・`min` 単位の正の値だけ受け付ける。

### 移行中の子スレッドへの返信

`normalizing` の間、未移行の子スレッドへ平文で返信すると、返信はDBで最上位の親へ付け替えられる（元の返信先は `reply_target_id`）。Web・TUI・デスクトップ・Androidは、送信応答の `parent_id` を使って、開いているスレッドと返信先を最上位スレッドへ移す（TUI・デスクトップは `client-core` の `send_message_resolved` が確定した親を返す）。

### 暗号化された入れ子

暗号文内の `parent_id` とDBの `parent_id` は復号時に照合される。暗号化された入れ子を平文と同じ手順で付け替えると復号できなくなるため、今回の移行では扱わない。`start` の検査に加え、バッチと親変更トリガーでも検出して失敗させる。失敗したバッチ全体が戻り、暗号文・親IDは保持される。直接の暗号化返信は従来どおり使える。検出時は移行を止め、認証対象の親IDと表示上の親IDを分ける設計を別途行う。検査の無効化や暗号文の書き換えでは回避しない。

### CNPGへの接続と実行

本番のクラスタは `zoo2` Namespaceの `zoo2-db`、DB名・所有者は `zoo2`。定義は [zoo2-infraのcluster.yaml](https://github.com/Zli-UoA/zoo2-infra/blob/main/kubernetes/apps/zoo2/database/cluster.yaml) と [運用手順](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/operations.md) を参照する。[通常の移行Job](https://github.com/Zli-UoA/zoo2-infra/blob/main/kubernetes/apps/zoo2/base/migrate/job.yaml) はAtlasのスキーマ適用だけを実行し、本スクリプトの `start`・`batch`・`finish` は実行しない。本変更のためのインフラ設定変更は不要。

運用者は対象拠点と通常バックアップ、互換バックエンドへの置換完了を確認し、`psql` がある作業環境で以下を実行する。previewでは専用context・Namespace・DBに読み替え、本番のSecretを使わない。

```sh
# ターミナル1: 書き込み先サービスへ転送する。終了時はCtrl-C。
kubectl --context zoo2-site1 -n zoo2 get cluster zoo2-db
kubectl --context zoo2-site1 -n zoo2 port-forward service/zoo2-db-rw 15432:5432
```

```sh
# ターミナル2: set -xを使わず、Secretを画面・履歴へ出さない。
export PGPASSWORD="$(kubectl --context zoo2-site1 -n zoo2 get secret zoo2-db-app -o jsonpath='{.data.password}' | base64 --decode)"
export DATABASE_URL='postgresql://zoo2@127.0.0.1:15432/zoo2?sslmode=require'
scripts/migrate-threads.sh status
scripts/migrate-threads.sh start
scripts/migrate-threads.sh batch
# statusと投稿レイテンシを確認し、必要な回数だけbatchを実行する。
scripts/migrate-threads.sh status
scripts/migrate-threads.sh finish
unset PGPASSWORD DATABASE_URL
```

クラスタの設定変更やJob作成は行わない。接続が切れたら、再接続後に `status` で確定済みの進捗を確認して再開する。

### 規模に応じた事前検証

本番に近い件数・返信分布の検証DBで、親なし投稿数、スレッド数、最大返信数・深さを記録する。`start` の祖先検査SQLを `EXPLAIN (ANALYZE, BUFFERS)` で測り、親なし投稿に祖先探索を行わないことと、設定した制限時間内に収まることを確認する。

バッチを先頭・半分・終盤まで進め、各時点の所要時間と投稿レイテンシを記録する。対象選択SQLの `m.id > COALESCE(migration_after, '')` に実際のカーソルを渡し、主キーのIndex Condが使われ、過去に完了した範囲を毎回走査しないことを確認する。`EXPLAIN ANALYZE SELECT migrate_thread_batch(...)` は実際に更新するため、本番で測定用に実行しない。巨大な1スレッドの更新時間と末尾での再走査は別に測る。所要時間は実データに依存し、固定時間では保証しない。

## 保持する情報と再実行

`reply_target_id` に元の返信先を保持し、本文、メッセージID、時刻、添付、リアクションを変えない。Homeの `parent_uri` と `reply_actor` は連合先に伝える実際の返信先として維持する。ローカル会話の親だけ正規化し、外部プロトコルの返信関係は書き換えない。

バッチは最上位親をロックし、参加者初期化・入れ子統合・表示データ再計算・`thread_migration_completed` の記録を同じトランザクションで確定する。投稿・参加・退出・アーカイブも同じ親で排他する。`thread_rollout.migration_after` に完了した親IDを記録し、次回はそれより後から主キー順に探索する。ロック中の親を飛ばさず、2秒でタイムアウトすればカーソルも戻して再試行する。複数バッチの同時実行は直列化する。末尾に達したらカーソルを戻し、移行中にカーソルより前へ追加された親を次の走査で拾う。`status` の残件と `finish` の検証で完了を判断する。途中失敗後は同じ `batch` を再実行できる。既存の参加判断、特に退出をバックフィルで上書きしない。子スレッドだけにある参加状態を統合するときは退出を優先する。

移行による親変更で新着・更新のアプリイベントを再送しない。統合前にOpenがあれば統合先もOpen、全てArchivedならArchivedを維持する。移行済みの旧 `thread=<子ID>` は最上位IDへ解決する。未移行の旧URLはバッチ完了まで従来の親を返す。

過去の退出履歴は存在しないため、初期参加者は生存投稿の投稿者から復元する。導入後の退出から厳密に保持する。

## 中断・切り戻し

バッチ実行を止めるだけで中断できる。完了済みスレッドは保持され、通常の新規投稿は正規化を続ける。`ready` で一覧に問題が出た場合は、対応バックエンドのまま `UPDATE thread_rollout SET phase='normalizing' WHERE id;` で参加フィルターを一時解除できる。データ・退出情報は削除しない。

`collecting` には戻さない。`normalizing` 開始後は、既存バッチが未実行でも非対応バックエンドへ戻さない。旧バックエンドはToastyのINSERT直後のモデルから変更前の `parent_id` を応答するため、DBトリガーの正規化結果と食い違う。旧リンク解決とINSERT後の再読み取りに対応したバックエンドだけを戻し先にする。退避列・完了テーブルは確認が終わっても本PRでは削除しない。

## クライアントと通信

Reactは既存のButton・ThreadPane・MessageRowを使い、スレッド内では既存の返信がある投稿だけ子スレッドを開ける。移行前の会話を辿る導線を保ち、空の子スレッドを作る操作は提供しない。移行後は返信数が0になり、この導線も消える。参加状態だけは既存Shapeにないため、閲覧中の1スレッドに限り共有API取得を行う。設定・利用者・IDでキャッシュを分離し、表示中60秒、非表示・未購読時停止、復帰・投稿後更新、取得中の更新集約を行う。操作結果は即時反映し、遅い旧応答を捨てる。本文をこのAPIで取り直さない。

TUIはスレッド選択中の `J` / `L` を参加・退出へ割り当てる。バージョンは0.12.0。Androidは既存のスレッドペインに参加・退出・再Openを加え、TUI・Android・GPUIも既存の子スレッドだけ開けるようにする。GPUIは子の返信がない場合だけキャッシュの親IDへ解決する。TauriのLeptos UIにはスレッド機能がないため変更しない。

## 検証

- `backend/tests/thread_participation_test.rs`: 参加と閲覧権限の分離、手動参加・退出、再Open、投稿再参加、再送、非公開権限。
- `backend/tests/thread_migration_test.rs`: 実スキーマでcollectingから段階移行、移行中投稿、元親保持、退出維持、再実行、イベント抑止、Home、履歴取り込み。
- 既存threads/archive統合テスト、Reactの一覧・参加状態単体テスト、ツリー遷移ブラウザーテスト、ThreadPaneのStorybook操作・a11y、`make tui-check`。
- schema.sqlから作ったDBと全マイグレーションから作ったDBを比較する。ER図はAtlas 1.3.0で生成する。

### 初回実装時の作業環境での結果（レビュー修正前）

- スレッド・アーカイブ・参加・段階移行のPostgreSQL統合テスト7件、Homeの統合テスト15件が成功。Homeには `ACTIVITYPUB_BASE_URL=https://zoo.test` のダミー設定を使用した。
- Reactの参加・一覧・ペイン単体テスト24件、ツリー遷移ブラウザーテスト11件、ThreadPaneのStorybook操作・a11y18状態が成功。型・lint・format、Storybook監査・ビルドも成功。390pxと1440pxの実ブラウザ表示で横はみ出しがないことを確認した。
- `make tui-check`、GPUIデスクトップの `cargo check` が成功。Androidは `buildCryptoAndroid` を除外してKotlinコンパイル・単体テスト・lintが成功。APK生成は `cargo-ndk` 未導入で未完了。リポジトリの `nix develop .#android` に必要なツールは定義済み。
- PostgreSQL 16の一時DBでスキーマ直接適用と全マイグレーション適用が成功。Atlasのテーブル比較、95個のDB関数定義比較、Atlas 1.3.0のER図一致を確認した。この環境にはDockerがないため、ER図は同じ生成スクリプトのDB接続部分だけを一時環境に差し替えて検証した。
- 一覧の主要SQLを、2,000スレッド中50参加・新しい1,950件が未参加という一時データで確認した。活動時刻と親IDの索引を使用し、参加判定の後に10件を取得した。実行時間は約1.7msだったが、本番規模・負荷での所要時間を保証する値ではない。
- 全体の `make test-e2e` は未完了。追加のアプリイベントテストは38件成功、Electricとオブジェクトストレージに依存する4件が失敗した。Slack取り込みの代表テストもローカルS3の接続拒否で実行を完了できていない。外部サービスを揃えたCI環境での再確認が必要。

本番DBへの適用・段階移行・デプロイは行っていない。

### レビュー修正後の確認

- 対象を絞ったバックエンド統合テスト9件が成功。collecting・normalizing中のHTTP操作、移行後の旧ID解決、Homeの500件超の対象保持、Home先行移行での元親保存、暗号化入れ子の停止を含む。
- 追加した `LegacyNestedReply` のStorybook操作・a11yが成功。フロントエンドの型・lint・format、Storybook監査も成功。
- TUI・GPUIの `cargo check`、Android共有部とアプリのKotlinコンパイルが成功。今回の確認では `buildCryptoAndroid` を除外し、全体テスト・APK生成は再実行していない。
- スキーマ直接適用と全マイグレーション適用、96個のDB関数定義の一致、ER図の再生成・一致、移行スクリプトの構文を確認した。本番DBでの実測・移行は行っていない。
