# チャンネルノートの取得・書き込み・期限切れ削除の探索量

## 設計時の変数と上限

無関係なノート履歴を N、対象チャンネルの有効ノート数を K（API 上限 500）、復元期限内の削除済みノート数を D、期限切れノートの削除バッチを B、cleanup が 1 回で調べるチャンネル数を C とする。K・D は固定し、N を 30 万件・100 万件へ増やして比較する。

| 処理 | 探索の上限 | 主な索引 |
| --- | --- | --- |
| 一覧 | 応答・メモリは 1 ページ K_page ≤ 200（既定 50）。SQL の走査は O(log N + K)（K ≤ 500、対象チャンネルの有効ノート範囲）。`idx_channel_notes_channel_position` は (channel_id, position) で id を含まないため、走査のあと (position, id) の Incremental Sort を挟む。after が空でないページも `CASE WHEN $2 = ''` は (position, id) への直接シークにならず、同じチャンネルの K ≤ 500 範囲内の走査になる | `idx_channel_notes_channel_position` |
| 削除済み一覧 | 応答・メモリは 1 ページ K_page ≤ 200（既定 50）。SQL の走査は対象チャンネルの 24 時間以内の削除済み D 行以内（O(log N + D)）。索引側の `deleted_at >= …- 24 hours` 条件が範囲を絞り、after 付きでも CASE はその範囲内のフィルタになる | `idx_channel_notes_expired`、`idx_channel_notes_deleted_at` |
| 単一取得 | O(log N + K)。generic plan でも K 行以内 | 主キー、`channel_id` 索引 |
| 作成 | 有効ノート数の確認が O(K)（上限判定）、最大位置の取得が O(log K + 1)、INSERT 1 行。E2EE trigger は `channels` の主キー | 有効ノートの部分索引、`Channel_pkey` |
| 更新・削除 | 主キーと `channel_id` で 1 行。generic plan でも K 行以内 | 主キー、`channel_id` 索引 |
| 復元 | 期限内の削除済み D 行以内から対象を探し、位置を後ろへずらす K 行以内 | `idx_channel_notes_expired`、有効ノートの部分索引 |
| 並べ替え | 更新は有効ノート K 行。API 全体は現在順・要求順のソートを含め最悪 O(K log K) | 有効ノートの部分索引 |
| 期限切れ削除 | 1 回の page は C=100 チャンネル。各チャンネルの存在確認は期限切れ索引で `LIMIT 1` の点検索（返却 0〜1 行）。削除は候補 B=100 行を読み、その中の未ロック行を削除する。ロック済み行は走査を延ばさず、次の回転で再訪する | `idx_channel_notes_expired`、主キー |
| 暗号化 trigger | 対象チャンネルの有効ノート K 行と復元可能ノートの存在確認。返却は 0〜1 行 | `idx_channel_notes_channel_updated` |

N はどの処理の探索上限にも入れない。無関係なチャンネルのノートは `channel_id` の索引範囲外であり、一覧・書き込み・削除の対象にならない。API は有効ノートが 500 件に達している状態の作成・復元を `409 Conflict` で拒否するため、K の最悪値は 500 件である。一覧・削除済み一覧は、1 要求あたりの応答件数・転送量・メモリが SQL の LIMIT で K_page ≤ 200（既定 50）に制限される一方、SQL 側の走査は対象チャンネルの範囲（一覧は有効ノート K ≤ 500、削除済み一覧は 24 時間以内の D 行）に収まる。`idx_channel_notes_channel_position` は (channel_id, position) で id を含まないため、一覧の走査は (position, id) の Incremental Sort を挟み、after が空でないページでも `CASE WHEN $2 = ''` は (position, id) への直接シークにならず、同じチャンネルの K ≤ 500 範囲内の走査になる。以下の 50 件計測は代表値で、一覧・復元は O(K)、並べ替えは更新 O(K) に加えてソート分の O(K log K) が上乗せされる。

## 検証結果

PostgreSQL 16.15、JIT 無効。ローカルの使い捨て DB で、Rust の統合テストから `EXPLAIN (ANALYZE, BUFFERS, TIMING OFF, FORMAT JSON)` を実行した。probe は本番と同じ位置にパラメーターを置き、対象 ID を `EXECUTE` の引数で渡す。単独 30 回は custom plan（バインド値で計画）、8 同時×30 回は `force_generic_plan`（値を参照しない計画）で、初回も捨てない。API は実際の Axum Router を in-process で呼び、認証・プール待ち・トランザクション・シリアライズを含む。HTTP ネットワークとブラウザー描画は含まない。debug build の参考値である。

### fixture

- 対象チャンネル: 有効 50 件（API 上限 K=500 の代表値）、空 0 件、削除済み 100 件（24 時間以内 50 件、期限切れ 50 件）。
- 無関係な履歴: 別チャンネルに N=300,000 / 1,000,000 件のノート。10% は期限切れ、残りは有効。
- API 書き込み用に UUID のノートを 16 件作成し、8 同時の更新・削除・復元で共有する。
- 計測対象の INSERT/UPDATE/DELETE は本番 trigger を有効にする。fixture の大量投入だけ trigger を外す。復元の probe は、対象の復元と位置ずらしの 2 つの UPDATE を 1 つの data-modifying CTE で同じトランザクション内で計測する。

### SQL の走査量と論理バッファ

行数は関係ごとの Actual Rows と除外行に loops を掛けた合計。論理バッファは hit+read。値は全サンプルの最大。

| 条件 | 30 万件 行数 / ブロック | 100 万件 行数 / ブロック |
| --- | ---: | ---: |
| 一覧（after なし・既定 50 件ページ） | 50 / 5 | 50 / 5 |
| 一覧（空） | 0 / 3 | 0 / 3 |
| 削除済み一覧（after なし・既定 50 件ページ） | 50 / 12 | 50 / 12 |
| 単一取得 | 1 / 4 | 1 / 4 |
| 作成位置の最大値 | 1 / 4 | 1 / 4 |
| 更新（trigger 込み） | 2 / 183 | 2 / 183 |
| 削除（trigger 込み） | 2 / 184 | 2 / 185 |
| 復元（位置ずらし込み） | 51 / 184 | 51 / 185 |
| 並べ替え 1 行（trigger 込み） | 1 / 184 | 1 / 183 |
| cleanup page（100 チャンネル） | 200 / 404 | 200 / 404 |
| cleanup delete（候補 100 行） | 150 / 505 | 150 / 506 |
| 平文 trigger の channels 点検索 | 1 / 4 | 1 / 4 |
| 暗号化 trigger の存在確認 | 1 / 378 | 1 / 361 |

N が 3.3 倍になっても、全条件で走査行数は同じで、論理バッファの差は索引の高さ・ページ配置による数ブロック以内だった。一覧・削除済み一覧の計測は after なしの先頭ページで、対象チャンネルの有効ノート（期限内の削除済みノート）がちょうど 50 件であるため、`LIMIT 50` がチャンネルの該当ノートを全て返す。続きのページ（after 付きカーソル）は計測していないが、応答は常に LIMIT で 200 行以内、走査も上記のチャンネル範囲（一覧 K ≤ 500、削除済み一覧は期限内 D）に収まる。更新・削除・並べ替え・復元の 183〜185 ブロックは、対象チャンネルの 50 件の索引・ヒープ更新と trigger の点検索であり、無関係な N 件の履歴ではない。復元の 51 行は、期限内の削除済み 50 件から対象を探し、位置ずらしで有効ノートを移動した合計である。cleanup delete の 150 行は、候補 100 行の索引走査とその中の未ロック 50 行の削除である。ロック済み行が先頭にあっても候補の読み取りは 100 行で止まり、スキップした行は次の回転で再び候補になる。暗号化 trigger の 361〜378 ブロックは、索引・ヒープのキャッシュ状態と対象チャンネルの 50 件の確認による観測値で、N が増えても同じ範囲に収まる。

実行計画の形は、一覧が `idx_channel_notes_channel_position` の Index Scan（チャンネルの有効範囲を位置順に読む）→ (position, id) の Incremental Sort → LIMIT、削除済み一覧が `idx_channel_notes_expired`（(channel_id, deleted_at, id)）の走査、cleanup の存在確認と削除候補が `idx_channel_notes_expired`、暗号化 trigger が `idx_channel_notes_channel_updated` の Index Scan で、いずれも対象チャンネルの索引範囲で止まる。一覧の after が空のページは Incremental Sort が LIMIT に 50 行流した時点で走査を止め、after が空でないページも custom plan では `position >= カーソル位置` が索引条件に入る一方、generic plan（8 同時）では `CASE WHEN $2 = ''` が Filter に残ってチャンネルの有効範囲の先頭から読み直す。いずれの形でも走査は対象チャンネルの K ≤ 500 範囲内であり、無関係な N 件を全件走査する Seq Scan はない。

### API 全体

各エンドポイントを単独 30 回、8 同時×30 回（240 サンプル）実行した。初回を含む。単位は ms。一覧・削除済み一覧は after なしの先頭ページ（既定の 50 件）で計測した。対象チャンネルは有効 50 件・期限内の削除済み 50 件のため、このページはチャンネルの該当ノートを全て返す。続きのページ（after 付き）はこの計測の対象外だが、ページングは 1 要求あたりの応答を LIMIT で 200 行以内に保つ。

| API | 規模 | 同時 | p50 | p95 | 最大 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 一覧 | 30 万 | 1 | 1.953 | 2.226 | 3.282 |
| 一覧 | 30 万 | 8 | 8.909 | 12.239 | 14.817 |
| 一覧 | 100 万 | 1 | 2.137 | 2.387 | 4.047 |
| 一覧 | 100 万 | 8 | 9.734 | 13.886 | 20.281 |
| 一覧（空） | 30 万 | 1 | 0.834 | 0.963 | 1.021 |
| 一覧（空） | 30 万 | 8 | 3.273 | 3.422 | 3.513 |
| 一覧（空） | 100 万 | 1 | 0.796 | 1.085 | 1.220 |
| 一覧（空） | 100 万 | 8 | 3.454 | 3.900 | 4.183 |
| 削除済み一覧 | 30 万 | 1 | 2.101 | 3.601 | 4.494 |
| 削除済み一覧 | 30 万 | 8 | 9.389 | 13.327 | 13.803 |
| 削除済み一覧 | 100 万 | 1 | 2.142 | 3.797 | 3.948 |
| 削除済み一覧 | 100 万 | 8 | 9.965 | 15.778 | 26.726 |
| 単一取得 | 30 万 | 1 | 0.856 | 1.000 | 1.065 |
| 単一取得 | 30 万 | 8 | 3.470 | 3.724 | 3.986 |
| 単一取得 | 100 万 | 1 | 0.814 | 1.082 | 1.115 |
| 単一取得 | 100 万 | 8 | 4.227 | 7.074 | 9.380 |
| 作成 | 30 万 | 1 | 0.840 | 1.090 | 1.263 |
| 作成 | 30 万 | 8 | 4.376 | 7.277 | 14.445 |
| 作成 | 100 万 | 1 | 1.060 | 1.751 | 2.358 |
| 作成 | 100 万 | 8 | 4.531 | 8.129 | 17.749 |
| 更新 | 30 万 | 1 | 0.838 | 1.210 | 1.452 |
| 更新 | 30 万 | 8 | 3.916 | 5.727 | 6.658 |
| 更新 | 100 万 | 1 | 0.943 | 1.752 | 1.848 |
| 更新 | 100 万 | 8 | 4.064 | 6.161 | 6.940 |
| 削除＋復元 | 30 万 | 1 | 1.719 | 1.954 | 2.008 |
| 削除＋復元 | 30 万 | 8 | 8.630 | 10.797 | 11.921 |
| 削除＋復元 | 100 万 | 1 | 2.295 | 5.105 | 7.604 |
| 削除＋復元 | 100 万 | 8 | 8.645 | 10.928 | 13.617 |
| 並べ替え（50 件） | 30 万 | 1 | 5.083 | 5.861 | 6.493 |
| 並べ替え（50 件） | 100 万 | 1 | 4.829 | 5.135 | 5.423 |
| 期限切れ削除 | 30 万 | 1 | 5.707 | 6.551 | 8.974 |
| 期限切れ削除 | 30 万 | 8 | 28.894 | 50.925 | 73.259 |
| 期限切れ削除 | 100 万 | 1 | 16.313 | 32.880 | 33.792 |
| 期限切れ削除 | 100 万 | 8 | 32.958 | 74.787 | 105.028 |

書き込みはチャンネル単位の `pg_advisory_xact_lock(hashtextextended('channel-notes:' || channel_id, 0))` で直列化される。8 同時の更新・削除＋復元・作成の p95 は 5〜16ms で、ロック待ちを含む。期限切れ削除は cleanup カーソル行の `FOR UPDATE` と削除バッチで 8 同時が競合し、p95 は約 51〜75ms、最大は 105.028ms だった。100ms は参考値であり、この最大値は隠さず記録する。デッドロック・タイムアウト・ディスク退避は全条件で 0。一覧・単一取得の 8 同時は読み取りのみで、N が増えても p50/p95 はほぼ変わらない。

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

`channel_notes_work_is_bounded_by_channel_notes_not_history` は対象チャンネルと無関係履歴を作り、SQL の走査行数・中間行・ソート入力・論理バッファを検証して `work.json` を保存する。各条件の絶対上限（ノート系 256 行、cleanup page 2,048 行）を超えるサンプルが 1 つでもあれば失敗する。`persisted_channel_notes_api_budget` は同じ DB の fixture を使い、単独 30 回・8 同時 240 回の API 時間と cleanup の実行時間を `api.json` に保存する。時間は記録するだけで合否に使わない。

二規模の比較は、それぞれの `work.json` に対して次を実行する。履歴に比例した走査・中間行・ソート・論理バッファの増加があると失敗する。

```sh
python3 infra/db/compare-channel-notes-work.py \
  performance-results/notes-300000 performance-results/notes-1000000
```
