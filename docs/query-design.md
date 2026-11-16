# SQLの設計と性能基準

## 必須の完了条件

アプリケーションが発行する各SQLを、**1回100ms以下**にする。読み取り、書き込み、ORM生成SQL、トリガー、同期・バックグラウンド処理を対象とする。DBマイグレーションのDDL自体はこの応答時間基準の対象外だが、ロック時間・書き込みへの影響を別に検証し、データ補完は小さいバッチへ分割する。

100msを超える場合は設計を修正し、再計測するまで未完了とする。既知の超過を残した状態で「性能改善完了」「マージ可能」と判断しない。既存コードや過去の計測結果も、自動的な適用除外にはならない。

時間は計画と実行を合算し、ロック待ちも含める。`EXPLAIN ANALYZE` のExecution Timeだけで判定しない。アプリから観測するSQLの所要時間も記録し、サーバー内の測定と差があれば転送・待ち・接続の影響を調べる。DB接続プール待ちとAPI全体の時間は別にも記録する。

複数のSQLへ分けた場合も、リクエスト全体のクエリ数・合計DB時間・API応答時間を検証する。100ms未満のSQLを大量に直列実行して基準を満たした扱いにしない。タイムアウトで遅い処理を中断するだけでも、成功する処理の軽量化にはならない。

## 実装前に計算量を書く

処理量の変数を明示する。例えば総投稿数をN、利用者の受信件数をR、フォロー数をF、ページ件数をK、対象投稿への返信数をDとする。探索、JOIN、ソート、集計、再帰、書き込みで更新する行・インデックスの数を、それぞれ評価する。

| 設計例 | 検討するオーダーと条件 |
| --- | --- |
| 一意キーによる取得 | B-tree探索O(log N)と返却行数。JOIN先も一意か確認する。 |
| 順序付きページ取得 | O(log N + K)を目指す。閲覧不可の候補を除外する場合は実際の候補数Cを別に置き、O(log N + C)とCの上限を示す。 |
| フォロー先ごとの取得 | O(F × (log R + K))など、Fを定数扱いせず、最大時の候補数・メモリ量も確認する。 |
| 全件ソート | O(N log N)。LIMIT Kがあっても前段が全件走査なら軽量なページ取得ではない。 |
| 深いOFFSET | ページが深いほど読み捨てが増える。keysetへ変更する。 |
| 正確な返信・リアクション集計 | O(D)を無条件で許容しない。Dが増えて100msを超えるなら、権限を守った事前集計・差分更新などへ変更する。 |
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

1. 再現可能な専用環境を定め、PostgreSQL・CPU・メモリ・設定・JIT・統計・キャッシュ状態を記録する。経過時間を安定して比較できない共有CIの結果だけで合格にしない。
2. 30万件と100万件以上で測る。対象ユーザーに履歴が集中する場合、多数のフォロー／参加先、大量の返信・リアクション、閲覧不可の候補、空の結果、深いページも含める。単純な均等分布だけで済ませない。
3. 初回の計画・取得と再実行を分け、キャッシュミスを含めて測る。通常の単独実行と8同時以上のアクセスを確認し、各条件で30回以上の値を採取する。初回の遅い値を外れ値として捨てない。
4. 各クエリの計画＋実行時間、アプリからのSQL所要時間、p50・p95・最大値を残す。**測定した各条件のすべての成功実行が100ms以下**を合格条件とする。平均・中央値・p95が低くても、100ms超過があれば未達とする。
5. 時間の検証に加え、実行計画の走査量・中間行数・再帰訪問数に回帰テストを置く。30万から100万へ増やしたときに、無関係な履歴の走査が増えていないことを確認する。
6. 100ms超過・タイムアウト・未計測の必要条件があれば性能検証を失敗として扱い、PRに未達項目を明記する。専用性能テストをCIへ組み込む場合も、超過を警告だけで通過させない。

`statement_timeout` は障害の波及を抑える手段として別に設定できるが、100msでエラーにすることと、100ms以内に正しい結果を返すことを混同しない。

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

[Supabase Postgres Best Practices](https://github.com/supabase/agent-skills/tree/main/skills/supabase-postgres-best-practices)（skill version 1.1.1）も参照する。Supabase専用の設定例をそのまま導入せず、本リポジトリの認証・接続先・移行方式へ適用できるか確認する。100ms基準は本リポジトリ独自の必須条件として維持する。

- **外部キー**：参照先の主キーだけでなく、参照元にも検索に使える先頭列のインデックスがあるか確認する。複合インデックスの途中に列があるだけでは不十分。削除済み行も含めた外部キー確認・CASCADEを検証し、生存行だけの部分インデックスで代用しない。[schema-foreign-key-indexes](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/schema-foreign-key-indexes.md)
- **式と検索列**：インデックス対象列へ接頭辞などの式を掛けて検索すると既存の主キーを使えないことがある。入力側を変換して主キーと比較するか、用途に合う式インデックスを選ぶ。[query-composite-indexes](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/query-composite-indexes.md)
- **繰り返し処理**：APIのN+1だけでなく、実行計画の高いloops値も確認する。同じリアクション・リポストの複数回集計、同一の親プレビューのJSON生成、ツリー情報の相関サブクエリを、ページ単位の集計・再利用・JOINへ置き換える。権限条件は共通化して維持する。[data-n-plus-one](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/data-n-plus-one.md)
- **汎用実行計画**：リテラルを埋めたEXPLAINだけでなく、パラメーター付きSQLのcustom／generic planも調べる。特にPL/pgSQLのバッチで「初回カーソルはNULL OR 範囲条件」とすると、generic planでは範囲探索を使えず、毎回先頭から走査することがある。初回と以降を別SQLにし、強制generic planでも深いページの走査量を検証する。[conn-prepared-statements](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/conn-prepared-statements.md)
- **ロック**：トランザクションを小さく保ち、外部通信を挟まない。複数ロックは実際の資源キーの同じ順序で取得する。ハッシュ化したadvisory lockなら、ハッシュ化前の名前順と実際のロック順を混同しない。[lock-short-transactions](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/lock-short-transactions.md) / [lock-deadlock-prevention](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/lock-deadlock-prevention.md)
- **接続と計測**：プール上限はレプリカ・ワーカー・常設LISTEN接続の合計で見積もる。接続先がtransaction poolingならprepared statementへの対応も確認する。Index Only Scanを期待する場合はheap fetchとvisibility map、統計・VACUUMの状態も確認し、INCLUDEを無条件に追加しない。[query-covering-indexes](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/query-covering-indexes.md) / [monitor-explain-analyze](https://github.com/supabase/agent-skills/blob/main/skills/supabase-postgres-best-practices/references/monitor-explain-analyze.md)
