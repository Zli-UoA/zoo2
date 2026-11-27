# SQLの設計と性能基準

## 必須の完了条件

アプリケーションが発行する各SQLには、入力件数の変数と実行計画に基づく**走査量・探索上限**を必須とする。読み取り、書き込み、ORM生成SQL、トリガー、同期・バックグラウンド処理を対象とする。総履歴Nに対して、表示対象Kやフォロー数Fに限定した処理になっているかを複数規模で検証する。実行計画1件だけをBig-Oの証明としない。

`EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` のActual Rows×Actual Loopsに除外行も加え、テーブルごとの合計走査量・中間行・ソート入力・再帰訪問数を確認する。LIMIT後の返却件数だけでは合格にしない。履歴量に比例した不要な走査や探索上限の超過は、設計を修正して再検証するまで未完了とする。

100msは参考値とし、実行環境による時間差だけでCIを失敗させない。初回の準備・計画・実行・ロック待ちを含む時間、SQL往復、接続待ち、API全体のp50・p95・最大値を残す。参考値の超過を隠さず、性能調査の判断材料にする。

複数SQLに分けた場合も、発行回数・合計の走査量・DB時間・API応答時間を確認する。タイムアウトによる中断や、キャッシュヒットだけの測定で探索量の問題を解消した扱いにしない。DB移行はDDLロック・書き込みへの影響を別に検証し、補完を小さいバッチへ分割する。

## 実装前に計算量を書く

処理量の変数を明示する。例えば総投稿数をN、利用者の受信件数をR、フォロー数をF、ページ件数をK、対象投稿への返信数をDとする。探索、JOIN、ソート、集計、再帰、書き込みで更新する行・インデックスの数を、それぞれ評価する。

| 設計例 | 検討するオーダーと条件 |
| --- | --- |
| 一意キーによる取得 | B-tree探索O(log N)と返却行数。JOIN先も一意か確認する。 |
| 順序付きページ取得 | O(log N + K)を目指す。閲覧不可の候補を除外する場合は実際の候補数Cを別に置き、O(log N + C)とCの上限を示す。 |
| フォロー先ごとの取得 | O(F × (log R + K))など、Fを定数扱いせず、最大時の候補数・メモリ量も確認する。 |
| 全件ソート | O(N log N)。LIMIT Kがあっても前段が全件走査なら軽量なページ取得ではない。 |
| 深いOFFSET | ページが深いほど読み捨てが増える。keysetへ変更する。 |
| 正確な返信・リアクション集計 | O(D)を無条件で許容しない。Dが増えるたびに全行を列挙するなら、権限を守った事前集計・差分更新などへ変更する。 |
| 再帰・グラフ探索 | 深さだけでなく分岐数・訪問件数・循環・ソート前の生成行数に上限を設ける。 |

これらは設計の目安であり、インデックスの有無やBig-Oだけで合格にしない。インデックスの高さ、ヒープ参照、本文の大きさ、権限判定、ロック、I/O、書き込み時の維持費用も実測する。

## クエリとインデックスを一緒に設計する

- 等価条件、範囲条件、並び順、同時刻の一意なタイブレーカーに合わせて複合インデックスを設計する。削除済み・非公開などを除く部分インデックスは、クエリがその述語を満たすことも確認する。
- インデックスを追加するだけで終わらず、`EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` で実際の利用、実行回数、走査行数、除外行数、ソート入力、ディスク退避、推定と実測の差を確認する。小さい固定テーブルのSeq Scanと、増え続ける履歴の全走査を区別する。
- 表示対象のキーを先に限定し、本文・JSON・絵文字・添付・集計は選択後に構築する。部品・行ごとのSQL発行を避け、ページ単位でまとめる。
- LIMITの前に行われる走査・ソート・集計・再帰が有界か確認する。返却上限と探索上限を別に記載する。途中でLIMITを適用しても、閲覧権限や並び順、必要な投稿を欠落させない。
- 集計キャッシュや非正規化を使う場合は、削除、公開範囲変更、フォロー解除、ブロック、同時書き込み、後着データ、旧アプリからの書き込みを含めて整合性を検証する。
- インデックスの追加では容量・INSERT／UPDATE／DELETEへの影響と重複インデックスも確認する。統計を更新し、必要なマイグレーションは既存のDB変更ルールに従う。

## 性能検証の合否

1. PostgreSQL・CPU・メモリ・設定・JIT・統計・キャッシュ状態を記録する。環境差のある経過時間だけで合否を決めない。
2. 30万件と100万件以上で、ページ件数・フォロー数・集計グループ数などをそろえて比較する。対象利用者への集中、大量の返信・リアクション・リポスト、非公開履歴、空の結果、深いページを含める。F・K・グループ数を変える検証も区別して行い、定数扱いしない。
3. 初回と再実行、単独と8同時以上を検証する。各条件30回以上を測り、初回を捨てない。準備・計画・実行、接続待ち、SQL往復、API時間を別に残す。
4. 各クエリについて入力から求めた行数・ソート・再帰の上限を検証する。行数はloopsと除外行を含め、関係ごとに合計する。全件走査、繰り返し展開、ディスク退避を検出し、必要なインデックスが実際に探索へ使われるか確認する。
5. 異なる規模の実測走査量・論理バッファアクセス（hit＋read）も比較する。総履歴が3.3倍になっても、同じK・F・グループ数で無関係な履歴の走査が比例して増えないことを必須とする。丸め・索引高・実行計画の切替を許す比較幅は、テストに明示する。単にSeq Scanを禁止する、推定コストや推定行数だけを見る、といった判定はしない。
6. 書き込みのEXPLAINはトリガー内の計画を省略するため、親の返却1行だけで合格にしない。入れ子の計画または親を含む論理バッファアクセスの増加を検証し、カウンター補完・公開範囲変更・競合時の整合性もテストする。ロック待ちとデッドロックも別に報告する。
7. 上限超過、不要な履歴走査、タイムアウト、必要条件の未検証はCIを失敗させる。100ms超過は参考として記録し、時間だけでは失敗させない。正確な件数・権限・ページ順序を犠牲にして上限を満たさない。

`statement_timeout` は障害の波及を抑える手段であり、探索量を減らす設計の代替にはしない。

## PRに残す情報

変更対象のクエリごとに、次の項目を記載する。SQLの一部分だけの計測をクエリ全体の結果として報告しない。

- 用途、クエリの識別子／ファイル、API当たりの発行回数。
- 入力件数の変数、計算量、最悪時の探索・返却・再帰上限。
- 利用するインデックスと、書き込み・容量への影響。
- fixtureの件数と偏り、実行環境、再実行コマンド、実行計画。
- 初回／再実行・単独／同時実行の計画＋実行、SQL所要時間、p50・p95・最大値、合否。
- クエリ数と合計DB時間、API全体の応答時間、権限・整合性の回帰検証。

Home検証の条件・結果・再実行方法は [home-performance.md](home-performance.md) を参照する。

## SupabaseのPostgreSQLベストプラクティスとの照合

[Supabase Postgres Best Practices](https://github.com/supabase/agent-skills/tree/main/skills/supabase-postgres-best-practices)（skill version 1.1.1）も参照する。Supabase専用の設定例をそのまま導入せず、本リポジトリの認証・接続先・移行方式へ適用できるか確認する。本リポジトリでは実行計画の走査量・探索上限を必須条件とし、100msは参考値として扱う。

- **外部キー**：参照先の主キーだけでなく、参照元にも検索に使える先頭列のインデックスがあるか確認する。複合インデックスの途中に列があるだけでは不十分。削除済み行も含めた外部キー確認・CASCADEを検証し、生存行だけの部分インデックスで代用しない。[schema-foreign-key-indexes](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/schema-foreign-key-indexes.md)
- **式と検索列**：インデックス対象列へ接頭辞などの式を掛けて検索すると既存の主キーを使えないことがある。入力側を変換して主キーと比較するか、用途に合う式インデックスを選ぶ。[query-composite-indexes](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/query-composite-indexes.md)
- **繰り返し処理**：APIのN+1だけでなく、実行計画の高いloops値も確認する。同じリアクション・リポストの複数回集計、同一の親プレビューのJSON生成、ツリー情報の相関サブクエリを、ページ単位の集計・再利用・JOINへ置き換える。権限条件は共通化して維持する。[data-n-plus-one](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/data-n-plus-one.md)
- **汎用実行計画**：リテラルを埋めたEXPLAINだけでなく、パラメーター付きSQLのcustom／generic planも調べる。特にPL/pgSQLのバッチで「初回カーソルはNULL OR 範囲条件」とすると、generic planでは範囲探索を使えず、毎回先頭から走査することがある。初回と以降を別SQLにし、強制generic planでも深いページの走査量を検証する。[conn-prepared-statements](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/conn-prepared-statements.md)
- **ロック**：トランザクションを小さく保ち、外部通信を挟まない。複数ロックは実際の資源キーの同じ順序で取得する。ハッシュ化したadvisory lockなら、ハッシュ化前の名前順と実際のロック順を混同しない。[lock-short-transactions](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/lock-short-transactions.md) / [lock-deadlock-prevention](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/lock-deadlock-prevention.md)
- **マイグレーションのロック分離**：既存行の検証を伴う `CHECK` は `ADD COLUMN` と `ADD CONSTRAINT ... NOT VALID` に分け、`VALIDATE CONSTRAINT` を `atlas:txmode none` の別トランザクションで実行して ACCESS EXCLUSIVE の保持時間を短くする（`20261206000000` / `20261206000001`）。
- **接続と計測**：プール上限はレプリカ・ワーカー・常設LISTEN接続の合計で見積もる。接続先がtransaction poolingならprepared statementへの対応も確認する。Index Only Scanを期待する場合はheap fetchとvisibility map、統計・VACUUMの状態も確認し、INCLUDEを無条件に追加しない。[query-covering-indexes](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/query-covering-indexes.md) / [monitor-explain-analyze](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/monitor-explain-analyze.md)

## 通知レベルとスレッド参加者への返信通知（#620）

### 実装前に決めた探索上限

変数: `M` = チャンネルの `channel_user` 行数、`A` = そのうち `notification_level='all'` の行数、`P` = 対象スレッドの `thread_participants` 全行数（退出済みを含む）、`L` = そのうち `left_at IS NULL` の行数（`L <= P`。SQL では除外せず Rust 側で判定する）、`T` = 1投稿の通知先候補数（`T <= M + P`）、`U` = `users` の総行数、`H` = 無関係な `thread_participants` 行数、`C` = 本文の `@username` 候補数。

| SQL / 呼び出し元 | 回数 | 索引と計算量 | 最悪の探索上限 |
| --- | --- | --- | --- |
| `ChannelUser::filter(channel_id)`（`backend/src/notifications.rs` の `create_message_notifications`） | 投稿1件につき1回 | `idx_channel_user_channel_id (channel_id, user_id)` の範囲走査。`M` 行を読み、レベルを Rust の `HashMap` に1回で集める | 走査 `<= M`、追加のユーザー問い合わせなし、ソート0、loops=1 |
| `notification_participants_sql()`（同ファイル。スレッド返信のときだけ） | 返信1件につき1回 | `thread_participants` の主キー `(thread_id, user_id)` を範囲走査し、退出済みの行も `left_at` と一緒に返す（親投稿者の退出判定に再利用）。`channel_user` は主キー `(user_id, channel_id)` を返却行すべてについて参照。`users` は読まない。マージ結合を選ぶ場合は `user_id` で並べ替える | `thread_participants` 走査 `<= P + 64`（並列 worker の端数）、`channel_user` 主キー参照 `<= P`（fixture では `L` 件だけが対象チャンネルに所属し、残りは無関係なメンバーシップ）、`users` 0行、ソート入力 `<= P`（外部ソートなし）、`channel_user` 側の Actual Loops `<= P + 1`、hit+read `<= 2P + 10P + 1024`（索引の段数の余裕）|
| bot・無効化・特権のないゲストの一括除外（同ファイル） | 候補が空でなければ1回 | `users` を `id IN (...)` で引く。ユーザーごとの問い合わせはしない（N+1なし）。`is_bot` / `is_deactivated` / `is_guest` を同じ行で取得し、bot・無効化は常に、ゲストは現在の `channel_user` に居ない場合だけ除く | `users` 参照 `<= T`、`loops=1` |
| リアクションのメンバー・レベル判定（`create_reaction_notification`） | リアクション1件につき1回 | `channel_user` の主キー1回で所属とレベルを同時に読む | 走査1行、`loops=1` |
| `current_level()`（`backend/src/routes/notification_settings.rs`） | GET/PATCH 1回につき1回 | `channel_user` の主キーと `channels` の主キーを JOIN。`deleted_at IS NULL` を同時に判定 | 各1行、`loops=1`、ソート0 |
| PATCH の UPDATE（同ファイル） | 値が変わるときは1回、同時更新の競合時は最大3回 | `channel_user` の主キー1行。`AND notification_level <> $3` で同値の二重更新を避け、0件なら現在値を読み直す | 更新1行（競合時は0行＋読み直し最大3回）、`txid` は変更時のみ |

通知行の INSERT は `T` 行まで（1ユーザー1件。`mention > reply > dm > channel_message` の順に `entry().or_insert()` して集約）。ID は Rust で生成し、`jsonb_array_elements` を使う1回の `INSERT ... SELECT` でまとめて書く（SQL 往復は1回）。双方向トリガーと `display_data` 投影は `FOR EACH ROW` なので行ごとに動く。`missed_call` と未読カウントの処理は変更しない。`all` の判定に本文の解析は使わず、暗号化チャンネルでは既存の暗号化プレースホルダをそのまま送る。

`M`・`P` はチャンネル／スレッドの人数で、`U`・`H` や投稿履歴の総量とは独立に上限を置く。`@username` の照合だけは従来どおり `C` 回の主キー参照（`C` は本文長に依存）で、今回の変更では増やさない。

新しいインデックスは追加しない。既存の `idx_channel_user_channel_id`、`channel_user` の主キー、`thread_participants` の主キーを使う。書き込み側の追加コストは `channel_user` 行への `notification_level` 1列（UPDATE は同値なら発生しない）だけで、新規参加時の DEFAULT 付与は既存の INSERT と同じ。"`all` の人数 A" で通知が増えても、探索量は `A` ではなく `M + P` に比例する。

### 検証条件と再現方法

- `SCALE_USERS=300000` と `1000000`、`SCALE_HISTORY`（省略時は `SCALE_USERS`）で `backend/tests/thread_participants_scale_test.rs` の `notification_participants_sql_does_not_scan_users` を実行する。移行済みの使い捨て DB で、`cargo test --manifest-path backend/Cargo.toml --test thread_participants_scale_test notification_participants_sql_does_not_scan_users -- --ignored --nocapture`。`SCALE_PLAN_DIR` に custom / generic の実行計画を保存する。
- 参加者 `P=10,000`（うち退出 500）、生存 `L=9,500`、`notification_level` は all / mute に半分ずつ、無関係な参加行 `H` を `SCALE_HISTORY` 件作る。`U` と `H` を30万→100万→（必要なら300万）に変えても、`P` が同じなら走査行数と hit+read が比例しないことを確かめる。SQL は退出済みも返すため、返却行数は `P`、`channel_user` 参照とソート入力の上限も `L` ではなく `P` になる。全員退出・空のスレッドも既存ケースで測る。
- 各ケースを初回と再実行、単独と8同時で30回以上測る。準備・計画・実行、接続待ち、SQL往復、API全体の p50・p95・最大値を別に残す。100ms は参考値で、時間だけで合否を決めない。上限超過、`users` / `thread_participants` / `channel_user` の Seq Scan と不要な走査、`P` を超えるソート入力や外部ソート、高い loops があれば未完了とする（fixture には無関係なメンバーシップも入れ、小さな表での Seq Scan を許さない）。
- 書き込みは、同値 PATCH で WAL が増えないこと（txid: null）、変更時が1行更新であること、参加・退出と通知設定の同時操作で権限・レベルが整合することを HTTP テストで確認する。`backend/tests/notification_settings_test.rs` と `backend/tests/notifications_test.rs` が検証先。

### M規模で未検証の項目

参加者クエリ（`notification_participants_sql()`）の30万／100万件の実測は上に記載した。一方、次の3つは**探索上限を設計として記載しただけで、M=30万／100万の実測は未実施**である。未検証を隠さず、PR 本文にもこの節を残す。

| SQL / 呼び出し元 | 記載済みの探索上限 | M規模の実測 |
| --- | --- | --- |
| `ChannelUser::filter(channel_id)`（`create_message_notifications`） | `idx_channel_user_channel_id` の範囲走査で `<= M` 行、loops=1、ソート0、ユーザーへの追加問い合わせなし | 未実施 |
| bot・無効化・特権のないゲストの一括除外（`users WHERE (is_bot OR is_deactivated OR is_guest) AND id IN (...)`） | `users` 参照 `<= T`、loops=1（宛先ごとの再問い合わせなし） | 未実施 |
| 通知行の一括 INSERT（`INSERT ... SELECT ... FROM jsonb_array_elements($1)`） | SQL 往復1回、挿入 `<= T` 行、行ごとの双方向トリガー・`display_data` 投影あり | 未実施 |

再現コマンド。移行済みの使い捨て DB で、対象チャンネルの `channel_user` 行数 `M` を30万と100万に変えてそれぞれ実行する（`M` の作り方は `backend/tests/thread_participants_scale_test.rs` の fixture を参照）。既存のスケール用 DB を流用するときは、対象チャンネルの割合が高すぎて Seq Scan に切り替わらないことと、`idx_channel_user_channel_id` が存在することを確かめる。索引のない DB や割合の偏った DB の値は、上の設計の検証には使えない。`:'recipients'` は (2) ではユーザーIDの配列、(3) では通知行オブジェクトの配列を渡す。

```sh
psql "$DATABASE_URL" -v channel_id='<対象チャンネルID>' -v recipients='[...]' <<'SQL'
\timing on
-- (1) ChannelUser::filter(channel_id)
EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
SELECT id, user_id, channel_id, notification_level
FROM channel_user WHERE channel_id = :'channel_id';

-- (2) 一括除外: users を id IN (...) で1回だけ引く
EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
SELECT id, is_guest, is_bot, is_deactivated FROM users
WHERE (is_bot OR is_deactivated OR is_guest)
  AND id IN (SELECT jsonb_array_elements_text(:'recipients'::jsonb));

-- (3) 一括 INSERT: 実際に書くため、ロールバックして計測する
BEGIN;
EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
INSERT INTO channel_notifications(id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at)
SELECT e->>'id', e->>'user_id', e->>'actor_id', e->>'channel_id', e->>'message_id',
       e->>'notification_type', (e->>'is_read')::boolean, (e->>'created_at')::timestamp
FROM jsonb_array_elements(:'recipients'::jsonb) AS e;
ROLLBACK;
SQL
```

測定する項目は (1)(2)(3) とも、`Node Type` と `Index Name`、`Actual Rows × Actual Loops`、`Rows Removed by Filter`、`Sort Method`、根节点の `Shared Hit Blocks + Shared Read Blocks`、`Execution Time` である。(1) は `M` の増加に伴って走査が増えるのが設計どおりで、`<= M` と1往復を超えないことを確認する。(2)(3) は `T` が同じなら `M` を3.3倍にしても走査行数・ブロック数・往復数が増えず、`users` の Seq Scan が出ないことを確認する。初回／再実行、単独／8同時を各30回以上測り、p50・p95・最大値と準備・SQL往復・API全体の時間を分けて残す。一括 INSERT は `EXPLAIN ANALYZE` にトリガー内の計画が出ないため、行ごとのトリガーと投影の費用は入れ子の計画または論理バッファの増加で別に確認する（「性能検証の合否」6 と同じ）。

### PR 本文用のメモ（クエリ設計・実行計画・探索上限・計測条件）

- 用途・識別子・回数: チャンネル投稿の通知先選択（`notifications.rs::create_message_notifications`、投稿1回につき `channel_user` 走査1・参加者1・除外判定1・INSERT 最大 `T` を1回）、スレッド返信の参加者通知（`notification_participants_sql()`、返信1回につき1）、リアクション判定（`create_reaction_notification`、1回）、設定の GET/PATCH（`notification_settings.rs`、1〜2回、競合時は読み直し追加）。
- 入力変数と上限: 上の表のとおり。走査 `<= M` + `<= P`、参照 `<= P`、`<= T`、返却通知 `<= T`、再帰0、参加者クエリのソート入力 `<= P`（外部ソートなし）。
- 索引と書き込み影響: 新規索引なし。`idx_channel_user_channel_id` と2つの主キーを使用。`notification_level` の DEFAULT は既存行を移行済みにし、同値 UPDATE を避ける。
- fixture・環境・再現コマンド: 前節のとおり。DB の版・CPU・メモリ・JIT・統計・キャッシュ状態と `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` の出力を PR に添付する。
- **測定結果（SQL のみ）:** PostgreSQL 16.15（Docker、同一ホスト、JIT off、最適化なしビルド）の使い捨て DB で `notification_participants_sql_does_not_scan_users` を実行した。`SCALE_USERS` / `SCALE_HISTORY` を30万と100万に変え、`P=10,000`（うち退出500）、`L=9,500`、対象チャンネル以外の `channel_user` を `SCALE_USERS` 行置いた。退出済みも返すよう SQL を変えたため再計測し、下の表にその結果を載せる。ms は SQL の往復を含む（API 全体ではない）。`users` の走査0、Seq Scan なし、外部ソートなし、上限内の走査・ブロックを custom / generic の両方で確認した。実行計画と JSON の計測値は `SCALE_PLAN_DIR` に保存した（`notification-custom-first.json` など）。

| `users` / 無関係な参加行 | 参加走査 (custom / generic) | `channel_user` 参照 (custom / generic) | hit+read (custom / generic) | 単独 p50/p95/最大 (ms) | 8同時 p50/p95/最大 (ms) | 合否 |
| --- | --- | --- | --- | --- | --- | --- |
| 30万 / 30万 | 10,000 / 10,000 | 9,500 / 10,000 | 700 / 59,716 | 9.9 / 11.0 / 11.4 | 60.2 / 65.9 / 70.9 | 合格（上限 参加 10,064 / `channel_user` 10,001 / ブロック 121,024 以内） |
| 100万 / 100万 | 10,000 / 10,000 | 9,500 / 10,000 | 703 / 59,720 | 9.6 / 9.7 / 9.9 | 57.7 / 61.4 / 69.3 | 合格（同上） |

参加行の走査は `P` で頭打ち。SQL が `left_at` で絞らなくなったため、`channel_user` の参照は generic plan で `P` まで増えたが、上限 `P + 1` の内側にある。PostgreSQL 16 では上位ノードのブロック数に子ノードの使用量が含まれるため、根节点の `Shared Hit Blocks` + `Shared Read Blocks` だけを 1 回数えた（走査ノードと `Sort` ノードの両方で足すと二重計上になる）。ブロック上限は `2P + BLOCKS_PER_KEY * P + 1024`（`BLOCKS_PER_KEY=10`、P=10,000 で 121,024）で、上の表の値と同じ式である。`U` を30万→100万に3.3倍にしても、`thread_participants` の走査は約10,000行で変わらず、無関係な参加履歴 `H` は読んでいない。初回と再実行の差は計測誤差の範囲。100ms は参考値で、8同時の最大70.9 ms は時間だけの合否判定に使わない。

- **未検証の残り:** API 全体（ルーター・接続待ち・JSON・ロック待ち）の p50/p95/最大、参加・退出と設定変更の同時実行時のロック待ち・デッドロック、本番 PostgreSQL 16.15 の実データでの `P` 分布。E2E 全体（`make test-e2e`）は未実行だが、対象のバックエンド統合テスト（`notification_settings_test` / `notifications_test` / `notification_storage_test` / `shape_proxy_test`）はローカルの Docker DB で通っている。ER 図は `infra/db/erd.sh` で再生成し、`erd.sh --check` も通っている。
