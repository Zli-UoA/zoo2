# チャンネルノートの取得・書き込み・期限切れ削除の探索量

## 設計時の変数と上限

無関係なノート履歴を N、対象チャンネルの有効ノート数を K（API 上限 500）、復元期限内の削除済みノート数を D、期限切れノートの削除バッチを B、cleanup が 1 回で調べるチャンネル数を C とする。K・D は固定し、N を 30 万件・100 万件へ増やして比較する。

| 処理 | 探索の上限 | 主な索引 |
| --- | --- | --- |
| 一覧 | 応答・メモリは 1 ページ K_page ≤ 200（既定 50）。本文を含まないため、応答サイズは K_page × メタデータ分。SQL は 2 つに分かれる: (1) `order_token` の集約 (ページ継続時の並び競合検出) がチャンネルの有効ノート K 行全体、(2) ページ読み取りが O(log N + K)。`idx_channel_notes_channel_position` は (channel_id, position) で id を含まないため、走査のあと (position, id) の Incremental Sort を挟む。after が空でないページも `CASE WHEN $2 = ''` は (position, id) への直接シークにならず、同じチャンネルの K ≤ 500 範囲内の走査になる | `idx_channel_notes_channel_position` |
| 削除済み一覧 | 応答・メモリは 1 ページ K_page ≤ 200（既定 50）。先頭ページと継続ページ（after 付き）で SQL を分け、継続ページでは `(deleted_at, id) > ($2::timestamp(3), $3)` を `CASE` で包まず直接 `WHERE` に置くため、custom plan・generic plan の両方でカーソル位置から索引走査を開始できる（O(log N + K_page)〜O(log N + D)） | `idx_channel_notes_expired`、`idx_channel_notes_deleted_at` |
| 単一取得 | O(log N + K)。generic plan でも K 行以内 | 主キー、`channel_id` 索引 |
| 作成 | 有効ノート数の確認が O(K)（上限判定）、最大位置の取得が O(log K + 1)、INSERT 1 行。最大 position が `i32::MAX` のときだけ、有効ノート K 行の `(position, id)` 連番への詰め直し (O(K log K)) が追加で走る。E2EE trigger は `channels` の主キー | 有効ノートの部分索引、`Channel_pkey` |
| 更新・削除 | 主キーと `channel_id` で 1 行。generic plan でも K 行以内 | 主キー、`channel_id` 索引 |
| 復元 | 期限内の削除済み D 行以内から対象を探し、位置を後ろへずらす K 行以内。ずらしで `i32::MAX` 行に当たるときだけ、有効ノート (復元済みを含む) K 行の `(position, (id <> $2), id)` 連番への詰め直しが置き換わる | `idx_channel_notes_expired`、有効ノートの部分索引 |
| 並べ替え | 現在順序の読み取りが ORDER BY 込みで K 行、更新は有効ノート K 行。API 全体は要求順のソートを含め最悪 O(K log K) | 有効ノートの部分索引 |
| 期限切れ削除 | 1 回の page は C=100 チャンネル。各チャンネルの存在確認は期限切れ索引で `LIMIT 1` の点検索（返却 0〜1 行）。削除は候補 B=100 行を読み、その中の未ロック行を削除する。ロック済み行は走査を延ばさず、次の回転で再訪する | `idx_channel_notes_expired`、主キー |
| 暗号化 trigger | 対象チャンネルの有効ノート K 行と復元可能ノートの存在確認。返却は 0〜1 行 | `idx_channel_notes_channel_updated` |

N はどの処理の探索上限にも入れない。無関係なチャンネルのノートは `channel_id` の索引範囲外であり、一覧・書き込み・削除の対象にならない。API は有効ノートが 500 件に達している状態の作成・復元を `409 Conflict` で拒否するため、K の最悪値は 500 件である。一覧・削除済み一覧は、1 要求あたりの応答件数・転送量・メモリが SQL の LIMIT で K_page ≤ 200（既定 50）に制限される一方、SQL 側の走査は対象チャンネルの範囲（一覧は有効ノート K ≤ 500、削除済み一覧は 24 時間以内の D 行）に収まる。有効ノート一覧はページ継続の競合検出に `order_token` を使うため、同じ文でチャンネルの有効ノート K 行全体を (position, id) 順に 1 回集約する。これは K に比例し N に依存しない。`idx_channel_notes_channel_position` は (channel_id, position) で id を含まないため、一覧の走査は (position, id) の Incremental Sort を挟み、after が空でないページでも `CASE WHEN $2 = ''` は (position, id) への直接シークにならず、同じチャンネルの K ≤ 500 範囲内の走査になる。以下の 50 件計測は代表値で、一覧・復元は O(K)、並べ替えは更新 O(K) に加えてソート分の O(K log K) が上乗せされる。

### 削除済みノートの保持量

復元期限内の削除済みノート数 D には、API の件数・サイズ上限を設けていない。有効ノートの K ≤ 500 は作成・復元時に強制するが、削除は編集・整理の正当な操作でも起きるため、削除を上限で拒否すると復元できるノートを失う。cleanup ワーカーは 60 秒ごとに、削除（`deleted_at`）から 24 時間を過ぎたノートを最大 100 行のバッチで非同期に物理削除する。高負荷や一時的な障害が続くと、削除から 24 時間を過ぎても物理削除が遅れ、D が増えることがある。下の fixture の有効 50 件・削除済み 100 件（24 時間以内 50 件・期限切れ 50 件）という規模は計測の前提であり、本番の削除件数を観測した値ではない。削除済み一覧の SQL は `(deleted_at, id)` のカーソルから索引走査を始め、1 ページの読み取りを LIMIT（≤ 200、既定 50）で抑えるため、D が増えても 1 要求の仕事は D に比例しない（O(log D + K_page)）。削除ループのような高頻度の削除が観測された場合は、削除済みノートの件数上限や rate limit を追加する余地がある。

## 検証結果

PostgreSQL 16.15、JIT 無効。ローカルの使い捨て DB（`initdb -E UTF8 --locale=C`）で、Rust の統合テストから `EXPLAIN (ANALYZE, BUFFERS, TIMING OFF, FORMAT JSON)` を実行した。probe は本番と同じ位置にパラメーターを置き、対象 ID を `EXECUTE` の引数で渡す。単独 30 回は custom plan（バインド値で計画）、8 同時×30 回は `force_generic_plan`（値を参照しない計画）で、初回も捨てない。API は実際の Axum Router を in-process で呼び、認証・プール待ち・トランザクション・シリアライズを含む。HTTP ネットワークとブラウザー描画は含まない。debug build の参考値である。有効ノート一覧の probe は本番と同じ SQL（メタデータ列 + `order_token` を出す `ordering` CTE + 外側の `ORDER BY page.position, page.id`）を使う。

### fixture

- 対象チャンネル: 有効 50 件（API 上限 K=500 の代表値）、空 0 件、削除済み 100 件（24 時間以内 50 件、期限切れ 50 件）。
- 無関係な履歴: 別チャンネルに N=300,000 / 1,000,000 件のノート。10% は期限切れ、残りは有効。
- API 書き込み用に UUID のノートを 16 件作成し、8 同時の更新・削除・復元で共有する。
- 計測対象の INSERT/UPDATE/DELETE は本番 trigger を有効にする。fixture の大量投入だけ trigger を外す。復元の probe は、対象の復元と位置ずらしの 2 つの UPDATE を 1 つの data-modifying CTE で同じトランザクション内で計測する。
- deep page の probe は、50 件の対象チャンネルで (1) 有効一覧は 40 番 (41 件目) の `(position, id)` を、(2) 削除済み一覧は期限内 46 件目の `(deleted_at, id)` をカーソルに使い、LIMIT 10 で続きのページを読む。カーソルの `deleted_at` は API が出力するのと同じミリ秒固定形式を fixture から読んで使う。

### SQL の走査量と論理バッファ

行数は関係ごとの Actual Rows と除外行に loops を掛けた合計。論理バッファは hit+read。`work()` が合否に使うのは「各条件の絶対上限（ノート系 256 行、cleanup page 2,048 行）」と「`Temp Written Blocks == 0`（ディスク退避なし）」で、custom plan (同時 1) と generic plan (同時 8) の両サンプルに適用し、`work.json` には deep page の 2 条件を含む全 15 条件を `/custom` と `/generic` の計 30 エントリに分けて記録する。論理バッファと中間行・ソート入力は記録値であり、合否の上限ではない (`compare-channel-notes-work.py` が 30 エントリすべてについて 2 規模間の比率で別途検証する)。

| 条件 | 30 万件 行数 / ブロック | 100 万件 行数 / ブロック |
| --- | ---: | ---: |
| 一覧（after なし・既定 50 件ページ） | 100 / 19 | 100 / 19 |
| 一覧（空） | 0 / 15 | 0 / 15 |
| 一覧 deep page（after = 40 番の位置・10 件） | 100 / 19 | 100 / 19 |
| 削除済み一覧（after なし・既定 50 件ページ） | 50 / 12 | 50 / 12 |
| 削除済み一覧 deep page（after = 期限内 46 件目・10 件） | 50 / 12 | 50 / 12 |
| 単一取得 | 1 / 4 | 1 / 4 |
| 作成位置の最大値 | 1 / 4 | 1 / 4 |
| 更新（trigger 込み） | 2 / 182 | 2 / 184 |
| 削除（trigger 込み） | 2 / 184 | 2 / 186 |
| 復元（位置ずらし込み） | 51 / 185 | 51 / 186 |
| 並べ替え 1 行（trigger 込み） | 1 / 192 | 1 / 184 |
| cleanup page（100 チャンネル） | 200 / 404 | 200 / 404 |
| cleanup delete（候補 100 行） | 150 / 506 | 150 / 505 |
| 平文 trigger の channels 点検索 | 1 / 4 | 1 / 4 |
| 暗号化 trigger の存在確認 | 1 / 355 | 1 / 337 |

N が 3.3 倍になっても、全条件で走査行数は同じで、論理バッファの差は索引の高さ・ページ配置による数ブロック以内だった。一覧の 100 行は、`order_token` の集約が対象チャンネルの有効ノート 50 行を読み、ページ読み取りが同じ 50 行 (ページ末尾で LIMIT が打ち切る前に索引範囲をなめる generic plan サンプルを含む) を読む合計である。deep page も同じ 100 行で、custom plan では `position >= カーソル位置` が索引条件に入るためページ側は 9 行だけ読むが、generic plan では `CASE WHEN $2 = ''` が Filter に残りチャンネルの有効範囲の先頭から読み直すため、上限値は最初のページと同じになる。一覧のブロックは旧計測 (本文を含む 1 文クエリ) の 5 から 19 へ増えたが、増分は集約の追加走査 (索引・ヒープの再訪問) であって N には依存しない (2 規模で同値)。削除済み一覧の deep page は最初のページと同じ 50 / 12 で、(deleted_at, id) の索引順で続きから読む。更新・削除・並べ替え・復元の 182〜192 ブロックは、対象チャンネルの 50 件の索引・ヒープ更新と trigger の点検索であり、無関係な N 件の履歴ではない。復元の 51 行は、期限内の削除済み 50 件から対象を探し、位置ずらしで有効ノートを移動した合計である。cleanup delete の 150 行は、候補 100 行の索引走査とその中の未ロック 50 行の削除である。ロック済み行が先頭にあっても候補の読み取りは 100 行で止まり、スキップした行は次の回転で再び候補になる。暗号化 trigger の 337〜355 ブロックは、索引・ヒープのキャッシュ状態と対象チャンネルの 50 件の確認による観測値で、N が増えても同じ範囲に収まる。並べ替え・作成の位置詰め直し (`i32::MAX` 回避) は有効ノート K 行の更新であり、この fixture では発生しない (位置が 0..49 のため)。

実行計画の形は、一覧が `idx_channel_notes_channel_position` の Index Scan（チャンネルの有効範囲を位置順に読む）→ (position, id) の Incremental Sort → `md5` 集約 (`ordering`) と、同じ Index Scan → Incremental Sort → LIMIT (`page`) を Nested Loop Left Join で結合した形 (空ページでも `order_token` 行を必ず返す LEFT JOIN) で、削除済み一覧が `idx_channel_notes_expired`（(channel_id, deleted_at, id)）の走査、cleanup の存在確認と削除候補が `idx_channel_notes_expired`、暗号化 trigger が `idx_channel_notes_channel_updated` の Index Scan で、いずれも対象チャンネルの索引範囲で止まる。一覧の after が空のページは `page` 側の Incremental Sort が LIMIT に 50 行流した時点で走査を止め、after が空でないページも custom plan では `position >= カーソル位置` が索引条件に入る一方、generic plan（8 同時）では `CASE WHEN $2 = ''` が Filter に残ってチャンネルの有効範囲の先頭から読み直す。いずれの形でも走査は対象チャンネルの K ≤ 500 範囲内であり、無関係な N 件を全件走査する Seq Scan はない。

### API 全体

各エンドポイントを単独 30 回、8 同時×30 回（240 サンプル）実行した。初回を含む。単位は ms。一覧・削除済み一覧は after なしの先頭ページ（既定の 50 件）で計測した。対象チャンネルは有効 50 件・期限内の削除済み 50 件のため、このページはチャンネルの該当ノートを全て返す。deep page（after 付き）の API 時間はこの計測の対象外だが、SQL probe が同じページ形式で 2 規模・両 plan で O(K) 以内を検証しており、ページングは 1 要求あたりの応答を LIMIT で 200 行以内に保つ。API 計測の合否は「全応答が 200 OK であること（認証・認可・直列化・trigger を含む）」だけで、時間は記録値であり合否に使わない。

| API | 規模 | 同時 | p50 | p95 | 最大 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 一覧 | 30 万 | 1 | 1.709 | 3.535 | 4.138 |
| 一覧 | 30 万 | 8 | 8.452 | 11.974 | 18.225 |
| 一覧 | 100 万 | 1 | 1.807 | 3.945 | 4.264 |
| 一覧 | 100 万 | 8 | 8.568 | 12.428 | 17.272 |
| 一覧（空） | 30 万 | 1 | 0.545 | 0.607 | 0.703 |
| 一覧（空） | 30 万 | 8 | 2.561 | 2.660 | 2.753 |
| 一覧（空） | 100 万 | 1 | 0.490 | 0.529 | 0.529 |
| 一覧（空） | 100 万 | 8 | 2.588 | 2.677 | 2.777 |
| 削除済み一覧 | 30 万 | 1 | 1.844 | 2.026 | 2.101 |
| 削除済み一覧 | 30 万 | 8 | 9.284 | 13.472 | 18.525 |
| 削除済み一覧 | 100 万 | 1 | 1.857 | 2.360 | 2.643 |
| 削除済み一覧 | 100 万 | 8 | 9.266 | 13.253 | 13.494 |
| 単一取得 | 30 万 | 1 | 0.610 | 0.706 | 1.198 |
| 単一取得 | 30 万 | 8 | 3.179 | 3.391 | 4.118 |
| 単一取得 | 100 万 | 1 | 0.562 | 0.698 | 0.958 |
| 単一取得 | 100 万 | 8 | 3.148 | 3.319 | 3.415 |
| 作成 | 30 万 | 1 | 0.740 | 0.857 | 1.058 |
| 作成 | 30 万 | 8 | 3.778 | 11.192 | 31.082 |
| 作成 | 100 万 | 1 | 0.792 | 1.960 | 1.978 |
| 作成 | 100 万 | 8 | 3.682 | 5.272 | 11.175 |
| 更新 | 30 万 | 1 | 0.698 | 0.883 | 1.200 |
| 更新 | 30 万 | 8 | 3.870 | 9.588 | 14.707 |
| 更新 | 100 万 | 1 | 0.779 | 0.947 | 0.976 |
| 更新 | 100 万 | 8 | 3.437 | 4.852 | 5.771 |
| 削除＋復元 | 30 万 | 1 | 1.510 | 5.729 | 6.553 |
| 削除＋復元 | 30 万 | 8 | 7.575 | 9.462 | 10.916 |
| 削除＋復元 | 100 万 | 1 | 1.574 | 1.853 | 2.151 |
| 削除＋復元 | 100 万 | 8 | 7.853 | 10.227 | 11.537 |
| 並べ替え（50 件） | 30 万 | 1 | 3.621 | 3.979 | 4.815 |
| 並べ替え（50 件） | 100 万 | 1 | 3.671 | 4.217 | 4.275 |
| 期限切れ削除 | 30 万 | 1 | 5.199 | 7.708 | 10.763 |
| 期限切れ削除 | 30 万 | 8 | 23.100 | 38.852 | 49.037 |
| 期限切れ削除 | 100 万 | 1 | 5.688 | 6.820 | 10.288 |
| 期限切れ削除 | 100 万 | 8 | 26.433 | 46.620 | 64.694 |

書き込みはチャンネル単位の `pg_advisory_xact_lock(hashtextextended('channel-notes:' || channel_id, 0))` で直列化される。8 同時の更新・削除＋復元・作成の p95 は 5〜12ms で、ロック待ちを含む。一覧は `order_token` の集約が加わったため旧計測 (1.953ms) から 1.709〜1.807ms (p50) と、debug build のばらつきの範囲で同等に収まっている。8 同時の一覧・削除済み一覧の最大は 13〜19ms で、読み取り専用のためロック待ちは含まない。期限切れ削除は cleanup カーソル行の `FOR UPDATE` と削除バッチで 8 同時が競合し、p95 は約 39〜47ms、最大は 64.694ms だった。100ms 未満は参考値であり、この最大値は隠さず記録する。デッドロック・タイムアウト・ディスク退避は全条件で 0。一覧・単一取得の 8 同時は読み取りのみで、N が増えても p50/p95 はほぼ変わらない。

## 再実行

空の使い捨て DB を規模ごとに用意し、全 migration を適用してから実行する。fixture は COMMIT するため、同じ DB で再実行しない。fixture の大量投入は `session_replication_role=replica` を使うため、計測ロールにはこの設定（スーパーユーザー、または該当設定を GRANT されたロール）が必要である。

```sh
psql "$ADMIN_DATABASE_URL" -c 'CREATE DATABASE notes_scale_300k'
atlas migrate apply --dir file://infra/db/migrations \
  --url "postgres://.../notes_scale_300k?sslmode=disable"
DATABASE_URL="postgres://.../notes_scale_300k?sslmode=disable" \
  SCALE_NOTES=300000 SCALE_PLAN_DIR=performance-results/notes-300000 \
  cargo test --manifest-path backend/Cargo.toml --test channel_notes_scale_test \
  channel_notes_work_is_bounded_by_channel_notes_not_history -- --ignored --nocapture
DATABASE_URL="postgres://.../notes_scale_300k?sslmode=disable" \
  SCALE_PLAN_DIR=performance-results/notes-300000 \
  cargo test --manifest-path backend/Cargo.toml --test channel_notes_scale_test \
  persisted_channel_notes_api_budget -- --ignored --nocapture
# 100 万件は別の空 DB を作り、SCALE_NOTES=1000000 で同じ 2 コマンドを実行する。
```

`channel_notes_work_is_bounded_by_channel_notes_not_history` は対象チャンネルと無関係履歴を作り、SQL の走査行数・中間行・ソート入力・論理バッファを検証して `work.json` に全 15 条件（deep page の 2 条件を含む）× 2 plan（`custom` と `generic`）の計 30 エントリを保存する（`deep.json` は作らない）。各条件の絶対上限（ノート系 256 行、cleanup page 2,048 行）と `Temp Written Blocks == 0` を超えるサンプルが 1 つでもあれば失敗する。`persisted_channel_notes_api_budget` は同じ DB の fixture を使い、単独 30 回・8 同時 240 回の API 時間と cleanup の実行時間を `api.json` に保存する。API 側の合否は応答が 200 OK であることだけで、時間は記録するだけで合否に使わない。

二規模の比較は、それぞれの `work.json` に対して次を実行する。`compare-channel-notes-work.py` は 30 エントリすべて（deep page の 2 条件を含む）を両規模で検証し、履歴に比例した走査・中間行・ソート・論理バッファの増加があると失敗する。

```sh
python3 infra/db/compare-channel-notes-work.py \
  performance-results/notes-300000 performance-results/notes-1000000
```
