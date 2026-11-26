# キャッチアップとチャンネル表示の性能計測

このブランチで行った改善の計測条件と生データ。数値はすべて手元の Linux 環境での値で、再現コマンドを併記する。

## 環境

- PostgreSQL 16（Docker コンテナ `zoo2-postgres-1`）
- Electric 1.8.1（`zoo2-electric-1`、既定 DB は `zoo`）
- backend は `DEV_AUTH_ENABLED=true`、`METRICS_PORT=0`、CORS に `http://127.0.0.1:4173` を指定
- frontend は `vp build --sourcemap` 後に `vp preview --host 127.0.0.1 --port 4173`
- ブラウザは Playwright の Chromium（headless、ロケール `ja-JP`、1440x900）
- 大規模 DB: `zoo`（users 100,107 件、messages 2,161,532 件）
- 小規模 DB: `zoo_perf`（users 2 件、1 チャンネル、messages 120 件、未読 100 件）
- Electric は DB ごとに `ELECTRIC_REPLICATION_STREAM_ID` を分ける（`zoo_perf` 側は `perftest`）

## キャッチアップ「既読にする」が有効になるまで

スクリプトは `frontend/scripts/bench-catchup.mjs` を使った。5 runs の中央値で比較する。

| 条件 | before | after |
| --- | --- | --- |
| 大規模 DB（`zoo`） | 16,951ms | 670ms |
| 小規模 DB（`zoo_perf`） | 812ms | 691ms |

大規模 DB の改善は `users` Shape を共同チャンネル member に絞った効果が主。小規模 DB の改善はキャッチアップのメッセージ取得を1ページ50件から150件にした効果。

再現例:

```sh
BENCH_BASE_URL=http://127.0.0.1:4173 node frontend/scripts/bench-catchup.mjs 5 label
```

`bench-catchup.mjs` は `http://127.0.0.1:4173/catch-up` を開き、`a[href="/catch-up"]` をクリックしてから「既読にする」が有効になるまでの `Date.now()` 差を測る。

## ページロード

perf-kit の既定スロットル（net x2、RTT 150ms、CPU 4倍）で3 runs を計測した。

| 変更 | LCP before → after |
| --- | --- |
| QR 検証部品の遅延読み込み | 4,905ms → 4,786ms（−2.4%、noise 98ms） |
| AppsDialog の遅延読み込み | 4,792ms → 4,694ms（−2%、5 runs、noise 96ms） |
| OPFS 早期オープンを取りやめた後の現状 | 4,943ms（元の 4,905ms に対して +38ms、noise 94ms で変化なし） |

OPFS の早期オープンは −169ms だったが、Service Worker の更新チェックより先に SQLite を開く順序の問題が見つかったため取りやめた。取りやめによりページロードの改善は相殺され、現状は誤差範囲内として扱う。

再現例:

```sh
CHROME_PATH=/usr/bin/google-chrome-stable npx @rolldown/perf-kit scan --app frontend --dist frontend/dist/client
```

## 実行計画

`docs/query-design.md` の「Users Shape の共同チャンネル絞り込み」を参照。100,107 件で 0.095ms / 11 blocks、1,000,000 件で M=2 のとき 0.139ms / 15 blocks。M=300,000 のとき 970ms / 903,415 blocks で、共同メンバー数に比例する。

## 未計測

- TUI、Android、desktop-tauri の実機検証
- 1,000,000 件で M が大きい場合の API 同時実行
- 本番相当の HTTPS / HTTP2 構成での API 応答時間
