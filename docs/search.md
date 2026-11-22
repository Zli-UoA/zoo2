# 全文検索

メッセージ・添付ファイル名・チャンネルノートを、サーバーで検索する。日本語を扱うため、PostgreSQL の拡張 [pg_bigm](https://github.com/pgbigm/pg_bigm)（2 文字ずつのインデックス）を使う。

- `pg_trgm` は使わない。本番の DB は C ロケールで、`pg_trgm` は ASCII 以外の文字を語として扱わず、日本語が引けない。
- 暗号化チャンネルは対象外。暗号化に切り替える前の平文の行も含めて、チャンネル単位で除く。暗号化チャンネルは、クライアントが読み込んで復号した範囲だけを手元で検索する（[e2ee.md](e2ee.md) の「検索」）。

## API

`GET /api/search`

| パラメーター | 説明 |
| --- | --- |
| `q` | 必須。空白で区切った語（最大 5 個、1 語 100 文字まで）。すべての語を含むものが当たる（AND）。英字の大文字・小文字は区別しない。`%` や `_` は文字どおりに扱う |
| `type` | `messages`（既定）・`all`（メッセージとファイル。クライアントの既定はこれで、明示して送る）・`files`・`notes`。省いたときの意味は変えない（`id` は常にメッセージの ID のまま） |
| `channel_id` | そのチャンネルだけ。見えないチャンネルなら結果は空 |
| `user_id` | 投稿者（ファイルはアップロードした人、ノートは作った人） |
| `after` / `before` | `YYYY-MM-DD`（UTC）。`after` の日を含み、`before` の日を含まない |
| `limit` | 1〜50（既定 20） |
| `cursor` | 前の応答の `next_cursor` |

応答:

```json
{
  "items": [ { "id": "…", "channel_id": "…", "channel_name": "…", "snippet": "…会議の資料…", "created_at": "2026-11-21T01:02:03.456", … } ],
  "next_cursor": "{\"m\":24323,\"at\":\"…\",\"id\":\"…\"}",
  "searched_until": "2025-12"
}
```

- 新しい順（`created_at`、同じ時刻は `id`）。各項目の `kind` は `message`・`file`・`note`。
- `all` は月ごとにメッセージとファイルの SQL を 1 本ずつ流し、(`created_at`, `id`) の新しい順に合わせる。同じ keyset を両方に当てるので、種類をまたいでも続きを取れる。
- **`items` が `limit` より少なくても、`next_cursor` があれば古い月がまだ残っている。** 1 回のリクエストで見るのは最大 12 か月で、その範囲で `limit` 件に届かなければ、見た所までで返す。クライアントは「さらに古い結果を検索」として続きを読む。`searched_until`（`YYYY-MM`）は、どの月まで見たかを表示するのに使う。
- `next_cursor` が `null` なら、見えるチャンネルの最も古いメッセージ（スレッドの返信も含む）まで見終わった。
- `snippet` は最初に当たった語のまわり（最大 160 文字、前後を省いたら `…`）。語の強調はクライアントで行う。
- メッセージは `parent_id`（スレッドの返信なら親の ID）、`author`（`display_data` の投稿者）、`emojis`（本文のカスタム絵文字）も返す。ファイルは `message_id`・`file_name`・`content_type`・`file_size`・`width`・`height`・`is_sensitive`、ノートは `title`・`updated_at` を返す。

### 見える範囲

- 削除されていない、暗号化されていないチャンネル。
- そのうえで、ゲストでない人の公開チャンネル、または所属しているチャンネル。ゲストは所属しているチャンネルだけ（`GET /api/messages` と同じ）。
- 削除したメッセージと、削除したメッセージの添付、削除したノートは出ない。

## インデックスと計算量

テーブルごとに、（作成月、本文）の複合 GIN を張る。作成月は `年*12+月` の式インデックスで、`btree_gin` の `int4` の opclass を使う。本文は `lower()` に `gin_bigm_ops`。

```sql
CREATE INDEX messages_search_month ON messages USING gin (
  ((date_part('year', created_at) * 12 + date_part('month', created_at))::integer) public.int4_ops,
  lower(content) public.gin_bigm_ops
) WITH (fastupdate = off) WHERE deleted_at IS NULL AND content <> '';
```

検索のトランザクションでは `plan_cache_mode = force_custom_plan` にして、値を埋めた計画だけを使う（generic plan では月と本文の GIN を使えないため。予算テストもこの計画を測る）。検索は新しい月から 1 か月ずつ、月の**等価条件**で引く（`backend/src/routes/search.rs` の `month_sql`）。GIN は月の posting と語の posting を突き合わせ、短いほうに合わせて読み飛ばす。そのため、1 か月分の SQL の索引の読み取りはその月の、語の 2 文字の組の posting（候補が 0 件でも読む）に、ヒープの走査は「その月の候補」（語の 2 文字の組をすべて含む行。再確認で除かれる行を含む）に限られ、どちらも総履歴には比例しない（1 文字の語のヒープの走査は、下のとおり「その月の行」）。範囲条件（`created_at >= …`）にすると、GIN は範囲の全行をビットマップに広げるので、使わない。

索引は `fastupdate = off` で作る。既定の pending list は、検索のたびに月によらず全体を読むので、上限がその月の量で決まらなくなる。pending list を持たないかわりに、書き込みのたびに索引へ直接入れる（メッセージ 1 件あたり、本文の 2 文字の組の数だけ）。予算テストは、`VACUUM ANALYZE` の後に書き込んだ行（書き込み直後の状態）も含めて測る。

変数: 総メッセージ数 N、見えるチャンネル数 V（全チャンネル数 C）、ある月の、その語のヒット数 H_m（見えないチャンネルの行も含む）、問い合わせの語を構成する 2 文字の組の数 Q、その月の、それらの組の posting の項目数 I_m（GIN が月の posting と突き合わせて読む量。候補が 0 件でも読む）、GIN がその月に返す候補の行数 G_m（語を構成する 2 文字の組をすべて含む行。再確認で除かれる行も含み、H_m ≤ G_m ≤ M_m）、ある月の対象の表の生きている行数 M_m、ページの件数 K、1 回のリクエストで見る月の数 B（最大 12）。

| SQL | 計算量 | 上限 |
| --- | --- | --- |
| 見える範囲（`SCOPE_SQL`） | O(C + V log N) | チャンネルを 1 回読み、見えるチャンネルごとに、生きている最古のトップレベルのメッセージ（`idx_messages_channel_unread`）と最古の返信（`messages_search_reply_floor`）を 1 行ずつ引く。親が削除されても返信は残り、取り込みではチャンネルの作成日時より古いこともあるため、返信も下限に入れる |
| 1 か月分（`month_sql`） | 2 文字以上の語だけ: O(Q log N + I_m + G_m + H_m log K)。1 文字の語を含む: O(M_m + H_m log K)。ファイルは候補ごとに親メッセージを確かめるので、さらに O(G_m log N)（1 文字の語を含むなら O(M_m log N)） | 2 文字以上の語だけなら、GIN の候補 G_m をヒープで再確認し（ヒットが 0 件でも G_m は読む）、ヒット H_m の上位 K+1 件をソートする。1 文字の語を含むと、その月の行 M_m をすべて確かめる。ファイルでは、LIMIT の前に候補ごとに親メッセージが削除されていないか（`EXISTS`）を主キーで引く。JOIN 先（`channels`、投稿者、添付の親メッセージ）は K 件分 |
| 1 リクエスト（`all` は月ごとにメッセージとファイルの 2 本の和） | O(C + V log N + Σ_{B か月} (Q log N + I_m + G_m + H_m log K、1 文字の語を含むなら M_m + H_m log K)。ファイルはさらに Σ G_m log N（または Σ M_m log N）) | SQL は最大 1 + B 回（`all` は 1 + 2B 回）。総履歴 N には比例しない |

**1 文字の語**: pg_bigm は 1 文字の語を、その文字を含むすべてのキーの部分一致で引く。部分一致は月の列との突き合わせで読み飛ばせないので、索引の読み取りが総履歴に比例してしまう。そこで 1 文字の語は索引の条件から外し（`(lower(本文) || '') LIKE …`）、月のキーで絞った行をヒープで確かめるだけにする。走査はその月の行数で頭打ちになり、総履歴には比例しない。

**最悪の場合**: とても多くの行に当たる語や 1 文字の語では、1 か月分の SQL の走査がその月のメッセージ数に近づく。それでもその月の行数で頭打ちで、履歴が増えても変わらない。H_m を減らすには、語を足すか、チャンネルや投稿者で絞る。`statement_timeout` には頼らない。

検証は `backend/tests/search_query_budget_test.rs`（CI は `.github/workflows/ci-search-query-budget.yaml`）。どの月も同じ密度（約 2 万件）にし、履歴は先月から過去へ延ばす（30 万件で約 14 か月、100 万件で約 49 か月）。そのうえで次を確かめる。

- 各 SQL の対象の表の走査が「今月のヒット数×3+64」以内（1 文字の語は「今月の行数×3+64」）。見えないチャンネルにだけ当たる語と、見えるチャンネルが 0 の閲覧者も測る。
- 30 万件と 100 万件で、表ごとの走査量とバッファが増えない（`infra/db/compare-search-work.py`）。
- 結合で捨てた行（`Rows Removed by Join Filter`）とソートの入力行（メモリ内のソートも）も、同じ上限で見る。
- API 1 リクエストごとに、SQL の回数・合計バッファ・DB 時間を `pg_stat_statements` で測る（手元の compose と CI で preload する）。該当なしのリクエストは月の SQL がちょうど 12 回（`all` は 24 回）、今月でそろう語は 1 回（`all` は 2 回）。合計バッファは 30 万件と 100 万件で増えない。
- 単独と 8 並列を各 30 回。API 全体の p50・p95・最大値も記録する。

## 拡張とデプロイ

- `pg_bigm` は trusted な拡張ではない。スーパーユーザーでない `zoo2` ロールは作れない。本番は CNPG の `Database` リソースで作る（Zli-UoA/zoo2-infra）。マイグレーション（`20261130000000_search_bigm.sql`）は `CREATE EXTENSION IF NOT EXISTS` なので、作成済みなら `zoo2` ロールでも通る。**この PR のマイグレーションより先に、zoo2-infra 側で DB のイメージを切り替え、拡張を作っておく。**
- `btree_gin` は trusted で、PostgreSQL に同梱されている。
- 拡張は `public` スキーマに置き、SQL では `public.likequery`・`public.gin_bigm_ops` と書く。テストは別スキーマを `search_path` にして `schema.sql` を流すため。
- インデックスは `CREATE INDEX CONCURRENTLY` で作る（書き込みを止めない）。

### 索引の作成に失敗したとき

`CREATE INDEX CONCURRENTLY` が途中で失敗すると、INVALID の索引が残る。Atlas（txmode none）は失敗した文から再開し、`IF NOT EXISTS` は名前だけを見て飛ばす。

- `20261130000000_search_bigm.sql` で残った INVALID は、次の `20261130000001_search_index_recheck.sql` が消して作り直すので、適用し直すだけでよい。
- `20261130000001_search_index_recheck.sql` の作成（後半の `CREATE INDEX CONCURRENTLY`）でも失敗した場合は、適用し直しても先へ進まない。Atlas は失敗した文から再開するので、作成の文は INVALID の索引を見て飛ばされ（成功として記録され）、次の最後の確認（`search index … is missing or invalid`）で止まる。以後は、再開しても確認の文だけが流れる。どちらで止まっていても、索引を手で作り直してから適用し直す。
  1. INVALID の索引を確かめる。対象は `messages_search_month`・`attachments_search_month`・`channel_notes_search_month`・`messages_search_reply_floor` だけ。

     ```sql
     SELECT c.relname, i.indisvalid
     FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
     WHERE c.relname IN ('messages_search_month', 'attachments_search_month',
                         'channel_notes_search_month', 'messages_search_reply_floor');
     ```
     （ない索引は、この結果に出ない。）
  2. 同じ索引を作っている処理がないことを確かめる（`SELECT pid, query FROM pg_stat_activity WHERE query ILIKE '%CREATE INDEX%'`。進行中の作成は `pg_stat_progress_create_index` にも出る）。
  3. `indisvalid = false` の索引だけを `DROP INDEX CONCURRENTLY <名前>;` で消す（有効な索引は消さない）。
  4. 消した索引と、1 で出なかった索引を、`20261130000001_search_index_recheck.sql` の `CREATE INDEX CONCURRENTLY IF NOT EXISTS …` の文をそのまま流して作り直す（定義を変えない）。
  5. 1 の SQL で、4 つがすべて `indisvalid = true` になったことを確かめる。
  6. マイグレーションを適用し直す（本番と preview は、マイグレーションの Job が作り直されたときに `atlas migrate apply` が流れる）。作成の文から再開した場合も、最後の確認から再開した場合も、索引が有効なので通る。
- 失敗の原因（ロック待ちの打ち切り、ディスク不足など）を先に取り除く。原因が残っていると、同じ所でまた失敗する。

## 手元と CI の DB

`pg_bigm` は PGDG の apt リポジトリにない。`infra/db/postgres/Dockerfile` で、`postgres:16.15-trixie` の上にソースからビルドする。ghcr.io/zli-uoa のイメージは private で、zoo2（public）の CI や手元からは pull できない。そのため、このリポジトリでビルドする。

- `infra/db/compose.yml` と `infra/compose.stack.yaml` は、このイメージをビルドして使う（`zoo2-postgres-bigm:16`）。
- Atlas の dev DB（`infra/db/atlas.hcl`）も同じイメージを使う。`make migrate-diff` の前に `make db-up`（ビルドされる）をしておく。
- 予算テストの CI と `ci-backend` の emoji のジョブは、service コンテナの代わりに、このイメージをビルドして `docker run` する。
- 今まで `postgres:16-alpine` で作ったローカルのデータは、`make db-reset` で作り直す（alpine（musl）と Debian（glibc）で文字列の並び順が違い、インデックスが合わなくなるため）。
- 本番（CNPG）と preview のイメージは Zli-UoA/zoo2-infra の `images/postgres-bigm` と `images/postgres-bigm-dev`。pg_bigm の版を上げるときは、3 つの Dockerfile をそろえる。

## クライアント

| クライアント | 入口 | 備考 |
| --- | --- | --- |
| Web（React） | 左のレールの検索、`/search?q=&type=&from=&in=&after=&before=` | 種類は「すべて」（既定。メッセージとファイル）・メッセージ・ファイル。送信者・チャンネル・期間（過去 7 日・30 日・1 年、または日付を指定）で絞り込む。続きは「さらに古い結果を検索」。本文の抜粋とファイル名の中で、検索語に当たった部分を `<mark>` で強調する（MFM の文字の部分だけ。コード・数式・メンションの中は強調しない） |
| Android（Compose） | チャンネル一覧のヘッダーの検索、`/search?q=&type=` | 種類は Web と同じ（既定は「すべて」）。入力が止まってから検索し、結果を押すとそのメッセージへ移動する（スレッドの返信はスレッドを開く。ファイルは添付したメッセージへ）。続きは「さらに古い結果を検索」で、「YYYY-MM まで検索しました」を出す。暗号化チャンネルが対象外であることを画面に出す。本文の抜粋とファイル名の中で、検索語に当たった部分を Web と同じ色で強調する（抜粋は MFM の文字の部分だけ。コード・数式・絵文字・メンションの中は強調しない。語の分け方は `SearchHighlight.kt` の `searchTerms`・`highlightSegments`）。検索の制御は `GlobalSearchController`、画面は `androidApp` の `SearchPage.kt`。絞り込みは必須ではない機能なので付けない（AGENTS.md「クライアントごとの機能の範囲」） |
| TUI | `S` / `Ctrl+F` | Tab で種類（すべて・メッセージ・ファイル）を切り替え、末尾で ↓ を押すと続きを読む。語に `from:@名前`・`in:#チャンネル`・`after:YYYY-MM-DD`・`before:YYYY-MM-DD` を書くと絞り込む（Slack と同じ書き方）。検索語に当たった部分を黄色の背景で強調する |
| デスクトップ（Tauri） | Web と同じ | 既定の Remote モードは React の画面を読むので、Web の検索がそのまま使える。開発用の Leptos の画面にはメッセージへ移動する手段がないので、検索は付けない |
| iOS | Zli-UoA/zoo2-ios で対応 | |

- ノート（`type=notes`）は、どのクライアントにもノートの画面がまだないので、クライアントからは使っていない。ノートの画面を作るときに、検索の種類に足す。
- 暗号化チャンネルの検索は、今までどおり各クライアントが読み込んで復号した範囲だけで行う。
