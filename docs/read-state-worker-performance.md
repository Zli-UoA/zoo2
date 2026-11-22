# 既読状態とバックグラウンドキューの探索量

## 設計時の変数と上限

総メッセージ数をN、対象チャンネル数をC、チャンネル・投稿者の組数をA、Slack同期対象数をT、保留中の合図数をW、取り込み履歴数をJ、表示更新キュー長をQ、ロック中・実行不可の候補数をLとする。時間は参考値とし、30万・100万件の実行計画と競合時の動作で検証する。

### 既読状態

元のSQLは `last_read_at IS NULL OR created_at > last_read_at` を相関条件に持つため、日時の範囲を索引条件にできず、チャンネルの過去履歴を読み直し得る。日時条件だけを直しても、自分の投稿ばかりなら他人の投稿を探すO(N)の走査が残る。

チャンネル・投稿者ごとの最新の生存ルート投稿日時を、同一トランザクションのトリガーで維持する。日時順の部分索引から未読境界より新しい投稿者を最大2件だけ取得し、その後で自分以外か確認する。1投稿者1行なので、最新が自分でも2件目までで判定できる。読み取りはO(C log A + 2C)を目標とし、N件の履歴を列挙しない。

キャッチアップ画面のために `unread_count` と `first_unread_at` を同じ一覧クエリの `?details=1` で返す。通常の10秒ポーリングと既読POSTの応答は詳細を要求せず、`channel_message_authors` の watermark だけを使う（`read_list_light` は `messages` 走査0行）。キャッチアップは画面を開いたときと明示的な更新のときだけ詳細を1回取得し、キューは固定する。詳細は `channel_message_authors` から未読境界より新しい他人の投稿者を最大 `UNREAD_AUTHOR_LIMIT = 100` 人選び、投稿者ごとに `messages_channel_author_latest`（`(channel_id, user_id, created_at DESC, id DESC)`）で未読件数を最大1001行まで数える。投稿者ごとの件数を合計して上限1000を超えた分と、投稿者が上限に達した場合は `unread_count_truncated` を立てる。最古の未読時刻は件数の上限とは独立に、未読のある投稿者を最大 `FIRST_UNREAD_AUTHOR_LIMIT = 1000` 人まで調べ、各投稿者の最古の1行を同じ索引の逆向き探索で取って最小値にする。投稿者の `latest_at` は代用しない。探索上限は加入チャンネル数Mと未読投稿者数Aに対して、件数が `M × min(A, 100) × 1001` 行、最古時刻が `M × min(A, 1000)` 回の索引参照で、自分の投稿や無関係な履歴Nには比例しない。LATERAL は `has_unread` のチャンネルだけに実行し、既読チャンネルは走査しない。同じ `created_at` の行に `id` の並びを要求しないため、既存索引で LIMIT の早期停止ができる。

書き込みは旧・新の組を一定順でロックし、`(channel_id,user_id,created_at DESC,id DESC)` の生存ルート投稿索引から最新1件を取り直す。本文・表示用JSONのみの更新では再計算しない。削除・復元、スレッドへの移動、投稿者・チャンネル・日時変更、ユーザー／チャンネル削除、並行書き込みと補完を検証する。既存行の補完はキー単位の小さいバッチで進め、先に差分トリガーを有効化する。

### Slack同期の取得

元の取得は実行可能時刻と合図による起動をORで結び、優先度の式を全候補で計算・ソートする。取り込み中かの照合も、終了済みを含むJ件の履歴に依存させない。

成熟した合図を先に探し、見つからなければ実行予定日時の索引から期限の来た候補を探す。合図の探索はW、通常の探索はO(log T + L)に限定する。取り込み中のジョブはteam_idを先頭にした部分索引で確認する。リース、SKIP LOCKED、合図のデバウンスと最短間隔、429の待機、暗号化チャンネルの除外、継続ページの公平性を維持する。

### 表示更新キュー

行ジョブと範囲ジョブには既に別々の `retry_at,kind,target,generation` 部分索引がある。追加索引だけで改善した扱いにせず、実際の利用とQに対する走査を確認する。ロック中の候補はLとして区別する。

空のキューを100msごとに2トランザクションで調べ続ける動作は、待機を段階的に延ばして問い合わせ回数を減らす。仕事が見つかれば直ちに待機をリセットする。上限1秒とし、通常メッセージの同期表示更新は維持する。メタデータ再投影の追加待機は最大1秒。新しいポーリングや接続は追加しない。

## 検証結果

PostgreSQL 16.16、AMD EPYC 9V45（5 vCPU）、メモリ約18GiBの共有開発環境で測定。shared_buffers=128MiB、work_mem=4MiB、effective_cache_size=4GiB、max_connections=100。計測接続はJITを無効化し、ANALYZE後の初回も捨てない。OS・DBキャッシュを消すcold-startの試験ではない。CIはUbuntu 24.04/PostgreSQL 16で別途再実行し、CPU・メモリ・設定・全条件のログとJSON計画をartifactへ保存する。

N=300,000/1,000,000のルート投稿に加え、同数のSlack取り込み履歴と表示更新キューを投入する。90%の投稿を同一利用者の3チャンネルへ集中させ、10%は無関係な投稿者・チャンネルへ置く。既読より古い他人の投稿、未読、同時刻、空結果も検証する。Slack履歴の1%は別チームの取り込み中、9%はファイル取得中、90%は完了済み。同期予定は300/1,000件。キューは将来の仕事N件と実行可能な行・範囲ジョブ各1件。

12条件それぞれ単独30回（custom plan）、8同時×30回（generic plan）、計6,480サンプルで検証した。投稿投入時だけ表示トリガーを省き、計測対象のINSERT/DELETEはすべての本番トリガーを有効にしている。書き込みサンプルはロールバックする。ロック待ちは実行時間に含まれる。同一投稿者の書き込みが競合する条件も含め、デッドロック・タイムアウトなし。

### 走査量と時間

行数はActual Rowsと除外行にloopsを掛けた関係ごとの合計。各サンプルの関係別走査・中間行・ソート入力は128以下、既読の要約走査は3チャンネル×2投稿者=6以下を必須にする。未読件数・最古時刻を返す `read_list`（`details=1`）は、チャンネルあたり最大100投稿者×1001行の走査を許すが、通常は未読投稿者の数だけ（fixtureでは3行）。`read_list_light` と `read_single` は `messages` 走査0を必須にする。さらに全サンプルの最大値を集め、100万件側の関係別行数・中間行・ソートは30万件側の2倍+64、論理バッファは2倍+256以内を要求する。索引の高さ・固定オーバーヘッドの増加は許すが、履歴3.3倍の走査は許さない。書き込みのAFTERトリガーは親Planに含まれないため、後述のauto_explainによる内部計画の検査も必須にする。

| 条件 | 走査行数 30万→100万 | 論理バッファ最大 30万→100万 | 100万件・8同時 計画+実行 p50/p95/最大(ms) |
| --- | ---: | ---: | ---: |
| `display_row_empty` | 0→0 | 3→4 | 0.023 / 0.068 / 0.927 |
| `display_row_ready` | 1→1 | 5→6 | 0.033 / 0.130 / 1.225 |
| `display_scope_empty` | 0→0 | 3→4 | 0.021 / 0.122 / 1.525 |
| `display_scope_ready` | 1→1 | 5→6 | 0.031 / 0.140 / 2.936 |
| `message_delete_latest` | 2→2 | 44→45 | 17.945 / 40.437 / 87.450 |
| `message_insert` | 1→1 | 485→488 | 82.274 / 143.637 / 324.451 |
| `read_list`（`details=1`） | 5→8（内 `messages` 3→3） | 50→54 | 0.112 / 0.220 / 2.034 |
| `read_list_light` | 4→8（`messages` 0→0） | 15→16 | 0.042 / 0.064 / 0.880 |
| `read_single` | 4→8（`messages` 0→0） | 6→7 | 0.024 / 0.037 / 1.238 |
| `slack_due_empty` | 0→0 | 4→4 | 0.089 / 0.457 / 5.174 |
| `slack_due_ready` | 10→10 | 33→33 | 0.122 / 0.225 / 2.228 |
| `slack_wake_empty` | 0→0 | 7→7 | 0.091 / 0.372 / 3.713 |
| `slack_wake_ready` | 14→14 | 39→41 | 0.161 / 0.687 / 11.345 |

親Plan上のソート入力は最大1行。旧既読SQLは30万件で870,006行、100万件で2,900,004行のmessagesを走査した。新SQLはどちらも0行で、要約は一覧3行・単一1行。旧Slack SQLは新しいactive-team索引を外した比較で、取り込み履歴30,000→100,000行と同期予定302→1,002行を走査した。新SQLは無関係な取り込み履歴を読まず、予約に必要なキーだけを照合する。

偏った分布の最悪側は `read_state_worker_scale_test` の `measure_skewed_details` で同じIgnoredテストの末尾に追加して測る。12チャンネル、各チャンネルに未読の他人120人（うち詳細の件数対象は上位100人）、各投稿者1001件のルート投稿、計約172.8万行のfixtureで、`details=1` は `messages` を1,202,400行走査し、論理バッファ1,263,014（100万件側は1,261,813）、単独の実測は約0.59秒（100万件側0.61秒）だった。上限式 `M × (min(A,100) × 1001 + min(A,1000))` は1,213,200行で、これを超えない。同じデータで軽量版は `messages` 走査0行・論理バッファ38、約0.64ms。Mに比例するため、加入チャンネルが非常に多い利用者では詳細取得が重くなるが、ポーリングには影響しない。この計測はCIのquery-budgetジョブでも実行する。

時間は本番の23.0s/2.62s/307msと同じ条件の比較ではない。本番の実行計画・負荷を取得できていないため、本番時間の改善率は主張しない。100msは参考値であり、競合INSERTの最大値が超えても隠さず記録する。SQLログには各接続の初回、接続確立、PREPARE往復、EXPLAIN往復を別々に残す。PREPARE往復は計画時間そのものではなく、計画+実行はEXPLAINのPlanning Time+Execution Time。SQL往復にはEXPLAINのJSON転送も含まれる。

### API全体

実際のRouterを使ったin-process GETの結果。認証、プール待ち、トランザクション、シリアライズを含み、HTTPネットワーク・ブラウザー描画は含まない。既定プール10、単独30回・8同時240回、初回も含む。接続初期化は別にログへ記録する。

| 規模・API | 同時数 | p50/p95/最大(ms) |
| --- | ---: | ---: |
| 300,000・一覧 | 1 | 3.015 / 5.224 / 106.101 |
| 300,000・一覧 | 8 | 3.630 / 7.068 / 218.596 |
| 300,000・単一 | 1 | 1.617 / 2.484 / 3.848 |
| 300,000・単一 | 8 | 4.747 / 5.544 / 6.183 |
| 1,000,000・一覧 | 1 | 1.191 / 2.618 / 108.145 |
| 1,000,000・一覧 | 8 | 3.039 / 3.996 / 173.352 |
| 1,000,000・単一 | 1 | 1.547 / 2.605 / 4.666 |
| 1,000,000・単一 | 8 | 4.968 / 6.027 / 6.864 |

既読一覧の変更SQLは1回、単一も権限確認後に1回。Slack予約は成熟wakeを1回、空ならdueを1回（最大2回）で、外部通信は予約トランザクションの外。表示更新のアイドル時は2回の予約を毎100msから最大1秒間隔へ減らし、定常時の予約回数は20回/秒から2回/秒になる。いずれも追加接続・クライアントリクエストは作らない。

## 移行・容量・運用上の範囲

新しい索引はCONCURRENTLYで構築し、Slackの旧available_at単列索引は新しい複合索引ができてから外す。新テーブルはPKのchannel_id先頭と逆向きuser_id索引で両方のFKを支える。日時索引・部分索引、prepared statement、短いトランザクション、FK索引についてSupabaseの指針とも照合した。

移行はトリガーを先に有効化し、チャンネル・投稿者キーを100組ずつkeysetで進め、各バッチでCOMMITする。先に要約をロックしてから新しいsource snapshotを読むため、補完と旧形式INSERTが競合しても更新を失わない。6,001組の既存データへmainから移行し、forced generic planでもすべてのmax(created_at)が一致すること、旧形式INSERTが反映されることを確認した。全migration適用とschema.sqlの構造一致、Atlas checksum、ERDも検証する。移行全体の既存履歴読み取りO(N)は一度だけ必要で、オンラインの既読判定へ持ち込まない。DDLは短いカタログロックを伴うため、運用時の待ち状況は監視する。

要約は過去に存在したチャンネル・投稿者の組Aに比例し、削除で空になった組も競合制御用にNULLで保持する。読み取り用部分索引にはNULLを入れない。親の削除ではCASCADEされる。100万件の長いテキストID fixtureでは、要約テーブル13MiB+索引合計47MiB、投稿者別最新索引312MiB、Slack新索引は合計約248KiBだった。索引・要約の書き込みと保存容量が増える代わりに、履歴の繰り返し走査を除く。

C/A/T/W/Lを無条件に定数扱いしない。既読一覧は参加チャンネルCに比例する。Slackのwakeは成熟した合図Wを探索・ソートし、期限順取得は期限の来た実行不可・ロック中候補Lを読み飛ばす。大量の合図や実行不可候補そのものをゼロコストにする変更ではない。SKIP LOCKEDでも返却1行と探索1行は同じではない。表示キューもロックされた候補Lは別に読む。無関係な投稿・完了済み取り込み・将来のキュー履歴N/J/Qに比例する走査を取り除くことを、このCIの保証範囲とする。

React・Android・TUI・デスクトップのDTO・取得経路・認証を変えず、共通バックエンドを改善する。クライアントコード・Storybook・TUI配布物の変更は不要。TUIは`make tui-check`を実行する。

## 計測前の統計

履歴の投入後は、計測対象DBの全テーブルに `ANALYZE` を実行します。履歴テーブルだけでなく、migrationで登録される固定カタログや、トリガーが参照する空テーブルも対象です。`app_event_types` の実件数と統計上の件数が一致することを検査し、ログに残します。

固定90行のイベント種別でも、統計がないDBでは索引で1行、統計があるDBではSeq Scanで90行という計画の違いが生じます。100万件の投入中だけautovacuumの自動解析が間に合うと、同じカタログを異なる条件で比較してしまいます。両規模の統計を明示的に更新して、この計測条件の差を取り除きます。走査量・中間行・論理I/Oの比較幅や、トリガー内の履歴走査を検出する判定は変更しません。

PostgreSQL 16.16の新規DBを規模ごとに用意し、全migration適用後に単独30回・8同時240回を再測定しました。両規模でカタログの実件数・統計件数・最大走査量はいずれも90です。DELETEの内部計画の合計走査は101→101行、論理ブロックは828→839、INSERTは120→120行・3052→3073ブロックで、12条件すべての規模間比較が通過しました。内部履歴走査・論理I/O増加・採取漏れを拒否する既存の回帰テスト4件も通過しています。

## 再実行

空の使い捨てDBへ全migrationを適用し、以下をそれぞれ実行する。fixtureはCOMMITするため既存DBでは実行しない。

```sh
SCALE_POSTS=300000 SCALE_PLAN_DIR=performance-results/300000-plans cargo test --manifest-path backend/Cargo.toml --test read_state_worker_scale_test -- --ignored --nocapture
# 別の空DBへDATABASE_URLを変更してから
SCALE_POSTS=1000000 SCALE_PLAN_DIR=performance-results/1000000-plans cargo test --manifest-path backend/Cargo.toml --test read_state_worker_scale_test -- --ignored --nocapture
python3 infra/db/compare-read-state-worker-work.py performance-results/300000-plans performance-results/1000000-plans
```

CI `Read state and worker query budget` は同じ手順を実行し、PRとマージキューで必須にする。整合性・権限・リース・再試行のテストは既存backend CIで実行する。

### トリガーの探索量を検査する経路

通常のEXPLAINのPlanには、AFTERトリガー内SQLの計画と、その処理のバッファ使用量は含まれません。親Planの比較だけでは、トリガー内で履歴を全件走査する回帰を検出できないため、書き込み計測にauto_explainの内部計画採取を追加しました。

計測用のsuperuser接続だけで `LOAD 'auto_explain'` と `log_nested_statements=on`、`log_min_duration=0`、`log_analyze=on`、`log_buffers=on`、`log_timing=off`、`log_format=json`、`sample_rate=1` を設定します。`client_min_messages=notice` と `log_level=notice` により、サーバーログと同じJSON計画を接続単位のNoticeから受け取ります。本番のアプリケーション接続やDB全体の設定は変更しません。

INSERT/DELETEの単独30回・8同時240回の全サンプルで、投稿者別最新日時を取得するSQLが採取されていることを必須にします。そのSQL以外のトリガー内SQLもすべて検査し、各計画の関係別走査・中間行・ソート入力を128以下、一時ブロック書き込みを0とします。今回の1投稿変更fixtureでは、採取する文を128以下、全計画を合計した関係別走査を512以下に制限します。

`work.json`の`nested`には、内部計画の関係別走査の合計、ソート・中間行の最大値、論理バッファの合計、採取文数を記録します。さらに、単独・8同時の全サンプルの最大値を取り、親Planと同じ規模間比較を必須にします。文数も100万件側が30万件側の2倍+16以内であることを検査します。入れ子の親子双方の計画を採るため、バッファの合計には重複計上があり、DB全体の物理I/O数を表す値ではありません。両規模を同じ方法で比較する検査値です。

初回の単独・8同時の計画は `message_insert-1-nested.json`、`message_insert-8-nested.json` と、同様のDELETEのファイルへ保存します。採取に伴う計画・転送の負荷は上の書き込み時間にも含まれます。実行時間は引き続き参考値です。

| 書き込み内部計画 | 全関係の走査合計 30万→100万 | 論理バッファ最大 30万→100万 | 最大採取文数 30万→100万 |
| --- | ---: | ---: | ---: |
| `message_insert` | 120→120 | 3052→3073 | 27→27 |
| `message_delete_latest` | 101→101 | 828→839 | 26→26 |

親が1行を返しても、内部で30万行を走査する計画はRustの負例テストで拒否します。`infra/db/test-read-state-worker-work.py` では、親の計測値を一定に保ったまま内部の走査行数または論理バッファだけを上限超過にした入力と、内部計画を欠落させた入力を比較スクリプトへ渡し、CIが失敗を検出することも検証します。

## CONCURRENTLY索引の構築失敗からの復旧

`20261128030000_read_state_worker_seek.sql` は `atlas:txmode none` で実行されます。Atlasは成功した文の進捗を保存するため、再試行時に適用済みの文をファイル先頭から実行し直す必要はありません。ただし、CREATE INDEX CONCURRENTLY自体が失敗した場合は、同名のINVALID索引が残り、その文の再試行が失敗することがあります。`IF NOT EXISTS`ではINVALID索引を作り直せません。

復旧は移行用の接続で行います。まず実行中の移行・再試行を止め、以下のSELECTで移行の進捗、構築中の索引、今回追加した索引の状態を確認してください。進捗のSELECTで履歴がない場合も、Atlasの履歴を書き換えずに失敗内容を確認します。

```sql
SELECT version,applied,total
FROM atlas_schema_revisions.atlas_schema_revisions
WHERE version='20261128030000';

SELECT pid,relid::regclass,index_relid::regclass,phase
FROM pg_stat_progress_create_index WHERE datname=current_database();

WITH expected(name) AS (VALUES
 ('messages_channel_author_latest'),('slack_channel_syncs_due_order'),
 ('slack_channel_wakes_due_order'),('slack_import_jobs_active_team'),
 ('channel_message_authors_latest'),('channel_message_authors_user'))
SELECT e.name,i.indisvalid,i.indisready,pg_get_indexdef(i.indexrelid)
FROM expected e LEFT JOIN pg_index i
 ON i.indexrelid=to_regclass('public.'||e.name)
ORDER BY e.name;
```

失敗した文と同じ名前で、`indisvalid=false`、かつ構築処理が残っていない索引だけを削除してください。有効な索引や別の移行の索引は削除しません。例えば最初の文の `messages_channel_author_latest` がこの状態なら、トランザクションの外で次を実行します。

```sql
DROP INDEX CONCURRENTLY IF EXISTS public.messages_channel_author_latest;
```

後続の索引で失敗した場合も、確認済みのINVALID索引の名前に置き換えて実行します。その後、同じmigrationディレクトリで通常の移行を再試行してください。今回の版の進捗を変更したり、SQLファイル全体を手動で流し直したりする必要はありません。

```sh
infra/db/migrate.sh migrate apply --dir file://infra/db/migrations --url "$DATABASE_URL"
```

再試行後は、上のSELECTで移行の `applied=total` と全6索引の `indisvalid=true` を確認してください。失敗の原因が残っている場合は、その原因を解消してから再試行します。

復旧手順は、使い捨てDBで最初のCREATE INDEX CONCURRENTLYを中断してINVALID索引を残し、削除後に同じmigrationを再開する方法で検証しました。再開後の `applied=total` と全6索引の有効化を確認しています。
