# 電話セッションとPBX参照の探索上限

`backend/tests/phone_query_budget_test.rs` は本番SQLをソースから読み取り、実PostgreSQLで `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` を実行する。返却件数だけでなく、走査行×loops・除外行・ソートのディスク退避を検査する。書き込みも実行し、FK検査を含む親計画の論理ブロックと競合を比較する。各計画はトランザクションをrollbackし、最初の計画をartifactへ残す。

## 変数と上限

Nは全回線・電話機・セッション数、Lは閲覧可能な回線数、Pは回線内電話機数（最大100）、Mは回線メンバー数（最大100）、Sはユーザーのセッション数（最大4）。セッション予約はuser → line → sessionの順でロックする。接続前の読み取りトランザクションを確定してから予約し、権限と更新版を再検証する。ネットワークI/O中はDBロックを保持しない。

| SQLの用途 | 探索・ソートの上限 |
| --- | --- |
| セッションの所有権・更新版・会員資格 | session PK → line PK・user PK・membership PK。O(log N)、各キー最大1行 |
| 同時接続数・期限切れ回収 | `(user_id,expires_at)` で対象ユーザーのみ。O(log N + S)、S≤4 |
| リース取得・更新・削除・内部ID保存 | session PK最大1行。UPDATE/DELETEの対象とFK参照先も定数 |
| PBX endpoint/auth/AOR | line PK、またはphone PK → line PK → user PK。O(log N)、各キー最大1行 |
| PBX内線の解決 | line PK確認後 `(line_id,extension)` で検索。該当最大P≤100。任意の全endpoint列挙は提供しない |
| 回線更新時の電話機ID検証 | `line_id` を索引の先頭にした検索で既存IDを一括取得。O(log N + P)、P≤100。電話機ごとの読み直しなし |
| 作成者の回線数上限 | `sip_lines_created_by_idx` で対象ユーザーの最大50回線のみ |
| ディレクトリー | owner索引・membership user索引から可視IDをUNIONし、line PKへ結合。無関係な共有回線を列挙しない。O(L log N + L×(P+M))、可視L件の並び替えO(L log L)。ページ/OFFSET・履歴展開なし |

ユーザー・回線・membershipの一意索引と、`phone_sessions_user_expiry`・`phone_sessions_line_idx`・`sip_phones_line_extension_idx`・`sip_lines_created_by_idx` を使用する。phone_sessionsは通話履歴ではなく期限付き登録で、同じユーザーが再接続する際に期限切れを回収する。

## リクエストとの関係

接続は認証共通処理のほか、最初のユーザー/回線確認2 SQL、予約5 SQL、内部ID保存1 SQL。外部I/OはJanus create・attach・registerの3回。通常コマンドは所有権確認1 SQLとJanus命令1回、切断は削除が1 SQL加わる。SSEは登録ごとに1本、内部Janus long pollも1本。10秒ごとの権限確認・リース更新は2 SQLとkeepaliveで、切断・失効・遅い受信者で停止する。PBX realtimeは参照ごとに1 SQL、内線解決のみ最大2 SQL。

APIの権限・同時接続・二重SSE・破棄・回線編集との競合は `backend/tests/sip_test.rs` で検証する。SQL単体の時刻をSIP登録や通話開始のAPI応答時間と扱わない。JanusのI/O時間・実回線の呼出時間・実機音声遅延は別の受け入れ項目。

## 測定と再現

各規模で、別ユーザーの回線・電話機・期限切れセッションをそれぞれ30万/100万件、対象に100電話機・4セッションを用意する。無関係なデータを単一所有者・単一回線に集中させ、APIの上限より大きい偏りでも対象検索を維持できるか確認する。所有者違い・存在しないセッション・空の内線・通常読み取り・書き込みを含め、単独30回と8接続×30回を実行する。初回PREPARE・計画・実行を捨てず、各回のSQL往復時間も保存する。再実行用 `PHONE_SCALE_REUSE=1` は同じ使い捨てfixture専用。

```sh
# それぞれ空のDBへ通常のmigrationを適用した後に実行する。
PHONE_SCALE_ROWS=300000 PHONE_SCALE_OUTPUT=/tmp/phone-300k.json cargo test --manifest-path backend/Cargo.toml --test phone_query_budget_test -- --ignored --nocapture
# DATABASE_URLを100万件用の別DBへ変更する。
PHONE_SCALE_ROWS=1000000 PHONE_SCALE_OUTPUT=/tmp/phone-1m.json cargo test --manifest-path backend/Cargo.toml --test phone_query_budget_test -- --ignored --nocapture
python3 infra/db/compare-phone-work.py /tmp/phone-300k.json /tmp/phone-1m.json
python3 infra/db/test-phone-work.py
```

専用CIは各規模のplan・全sample・CPU/メモリ/PostgreSQL設定と比較結果をartifactへ残す。関係ごとの走査上限をRustテストで検査し、規模間比較は論理ブロックが `2×小規模+64`、関係ごとの走査が `2×小規模+8` を超えたら失敗する。索引高・丸め・競合を許し、3.3倍の履歴比例増加は許さない。比較器にはブロック比例増加・走査比例増加・同時実行sample不足を拒否する回帰テストを付ける。時間の100msは参考値で、時間だけでは合否を決めない。

開発環境はPostgreSQL16、cgroup CPU上限4 CPU、メモリ約18GiB。Android/Rust/Webのビルドと共有して測定しており、OSキャッシュは強制破棄していない。CPU待ち・同じ行の書き込み競合を含む外れ値を隠さず、計画/実行/往復を分けて記録する。実運用環境の性能保証ではない。

## 開発環境での結果

24条件を各規模6,480回測定し、全条件で探索上限と規模間比較が通った。回線・電話機・セッションに加え、無関係な会員登録も30万/100万件へ増やし、個人所有者・共有メンバー・閲覧不可・存在しない対象を確認した。対象ユーザーの可視回線は2件、回線内電話機は100件、接続は4件。各関係の最大走査は両規模とも同じで、phone_sessions 5行・sip_lines 3行・users 2行・sip_phones 100行・membership 2行だった。電話機が一つの回線へ偏っていても、一覧の子集計は索引順で100件以内を取り、その範囲で名前順に整列する。

下表は24条件をまとめたp50/p95/最大(ms)。SQL往復にはPREPARE・EXPLAIN・JSON転送・rollbackを含み、HTTP API時間ではない。

| 無関係な各テーブルの行数 | 接続数 | 計画 ms | 実行 ms | SQL往復 ms |
| --- | ---: | --- | --- | --- |
| 300,000 | 1 | 0.128 / 1.492 / 37.832 | 0.122 / 1.013 / 53.569 | 1.776 / 10.301 / 152.444 |
| 300,000 | 8 | 0.162 / 2.899 / 133.535 | 0.126 / 2.859 / 133.925 | 5.402 / 23.837 / 345.548 |
| 1,000,000 | 1 | 0.156 / 2.505 / 19.519 | 0.127 / 2.559 / 28.457 | 3.884 / 14.002 / 62.619 |
| 1,000,000 | 8 | 0.153 / 4.497 / 68.313 | 0.123 / 5.000 / 53.506 | 12.173 / 36.921 / 148.175 |

100msを超える往復の外れ値も含めて記載した。専用環境での遅延測定や実機音声の確認を代替する値ではない。


### API全体

実Routerで回線一覧・DTMF命令・PBX endpoint参照を、それぞれ単独30回・8同時240回測定した。開発用認証、プール待ち、全SQL、レスポンス生成を含む。DTMFは実HTTPでfake Janusへ接続し、PBX参照は専用Bearerの検証も含む。JWT/OIDC、インターネット通信、SIP登録、音声品質の測定ではない。API sampleの欠落も比較器で拒否する。

| 規模 | API | 接続数 | p50 / p95 / 最大 ms |
| --- | --- | ---: | --- |
| 300,000 | `directory` | 1 | 15.434 / 52.705 / 449.671 |
| 300,000 | `directory` | 8 | 95.933 / 726.129 / 923.963 |
| 300,000 | `command` | 1 | 9.321 / 24.158 / 47.130 |
| 300,000 | `command` | 8 | 65.547 / 231.584 / 827.591 |
| 300,000 | `pbx-endpoint` | 1 | 3.931 / 8.152 / 296.138 |
| 300,000 | `pbx-endpoint` | 8 | 23.990 / 127.401 / 761.189 |
| 1,000,000 | `directory` | 1 | 18.334 / 45.740 / 739.502 |
| 1,000,000 | `directory` | 8 | 85.790 / 200.460 / 1474.065 |
| 1,000,000 | `command` | 1 | 11.967 / 32.384 / 46.788 |
| 1,000,000 | `command` | 8 | 66.257 / 130.091 / 274.188 |
| 1,000,000 | `pbx-endpoint` | 1 | 4.182 / 15.150 / 20.684 |
| 1,000,000 | `pbx-endpoint` | 8 | 22.029 / 49.435 / 101.724 |
