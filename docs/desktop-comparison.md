# デスクトップクライアントの比較

zoo2 のデスクトップクライアントは 3 通りある。GPUI 版 (`desktop/`)、Tauri (a) Leptos 版、Tauri (b) remote React 版 (`desktop-tauri/`。(a) と (b) は `ZOO_DESKTOP_UI=leptos|remote` で切り替える)。この文書は、残りの画面を GPUI で作り続けるかを決める材料として、同じ条件で測れたものと、測れなかったものを並べる。

測っていないものは「未計測」と書き、理由を添える。推測の数字は入れていない。

## 測り方

- 環境: Xvfb (1280x800) + openbox + lavapipe (CPU の Vulkan)。`nix develop` の中で `scripts/desktop-bench.sh` を動かした。実機の GPU ではない。
- 画面: openbox は、飾りなし・最大化の設定 (`desktop-screenshot.sh` と同じ) にした。root ウィンドウ全体がクライアント領域になり、WM のタイトルバーや枠は測りに入らない。
- 対象の画面: **ログイン前の画面**。backend に接続せず、AuthZ のログインもしていない (`ZOO_API_BASE_URL=http://127.0.0.1:9`)。
- ビルド: GPUI は `cargo build --release` (`strip = true`)、Tauri は `trunk build --release` と `cargo build --release`。どれも release で、debug の値は使っていない。測ったコミットは `bf4c983c` (feat/desktop-gpui-resident。その後のコミットはドキュメントと計測スクリプトの整理だけで、アプリのコードは同じ)。
- 回数: 各 5 回。表は中央値で、各回の生の値は下に載せる。
- 最初の描画: 起動から、root の絵の輝度の標準偏差が閾値 (0.005) 以上になる最初の絵が出るまで。`xwd` で root を撮って比べているので、撮るのにかかる時間のぶん (実際の間隔は 83〜99 ms) 粗い。**差が測る間隔以下のものは、差として扱わない。** 目安であり、フレーム単位の計測ではない。
- 「絵が落ち着くまで」も測ろうとしたが、入力欄のキャレットなどで 8 秒間変わり続け、判定できなかったので載せない。
- RSS: 起動 8 秒後の、プロセスと子プロセス全体の VmRSS の合計 (共有ライブラリのページは重複して数えるので、実メモリより多い)。本体 (起動したプロセスだけ) と、Pss (共有ページを按分した合計。`smaps_rollup`) も並べる。
- CPU: 起動 8 秒後から 10 秒間の、プロセス全体の使用率 (1 コア = 100%)。

## 比較表

| 項目 | GPUI | Tauri (a) Leptos | Tauri (b) remote React |
|---|---|---|---|
| 最初の描画まで (中央値、範囲) | 230 ms (214〜251 ms) | 191 ms (176〜381 ms) | 226 ms (215〜243 ms) ※1 |
| RSS 合計 | 197 MiB (1 プロセス) | 335 MiB (3 プロセス) | 336 MiB (3 プロセス) ※1 |
| RSS 本体 | **197 MiB** | 152 MiB | 153 MiB ※1 |
| Pss 合計 | 194 MiB | 192 MiB | 193 MiB ※1 |
| CPU アイドル時 | 0.2% | 0.0% | 0.0% ※1 |
| CPU スクロール時 | 未計測 ※2 | 未計測 ※2 | 未計測 ※2 |
| 実行ファイル | 58.2 MiB (release、strip 済み。Noto Sans JP 4 ウェイト約 22 MiB を含む) | 19.2 MiB (CI の deb 内は 19.5 MiB) | (a) と同じ実行ファイル |
| 配布物 (bundle) | 未作成 ※3 | deb 7.7 MiB、AppImage 83.3 MiB (CI の成果物) | (a) と同じ |
| 実行時の依存 | libxkbcommon、libxcb (X11) / libwayland、Vulkan ローダー (と GPU のドライバー)、fontconfig、D-Bus (通知・トレイ) | WebKitGTK 4.1 (と GTK など)、libayatana-appindicator (トレイ)、D-Bus | 同左 + 配信元の frontend |
| IME | 未計測 ※4 | 未計測 ※4 (WebKitGTK の IME) | 未計測 ※4 (WebKitGTK の IME) |
| 通知 | notify-rust (D-Bus)。Linux はクリックで前面に出し、そのチャンネルと投稿を開く。macOS は出すだけ | notify-rust。同じ判定 (`client-types::notify`)。クリックでチャンネルと投稿を開く | 同左 |
| トレイ | Linux は ksni、macOS は tray-icon (macOS は未検証)。メニューは「開く」「終了」 | Tauri のトレイ。メニューは「開く」「終了」 | 同左 |
| 閉じたときの挙動 | X11 は最小化してトレイに常駐。Wayland は終了 (戻す手段がない) | トレイに常駐 | 同左 |
| 単一インスタンス | Unix ソケット + ロックファイル (flock) | tauri-plugin-single-instance | 同左 |
| スクリーンリーダー | 非対応 (gpui 0.2.2 は AccessKit などの対応がない) | WebKitGTK の AT-SPI (未検証) | 同左 (未検証) |
| 日本語フォント | Noto Sans JP 4 ウェイトを同梱 (約 22 MiB)。Geist は静的な 4 ウェイトを同梱 | OS のフォント | OS のフォント (frontend が指定する Web フォント) |

※1 (b) は backend (frontend の配信元) に届かない状態で測ったので、WebKit のエラーページを描いている。React の読み込みと描画は含まない。(b) の数字は参考にならず、実際の起動時間と RSS は未計測 (backend と AuthZ が要る)。
※2 スクロールの CPU は、ログインしてメッセージの入った backend につなぐ必要がある。AuthZ のログインと backend を使わない条件で測ったので、計測していない。
※3 GPUI 版は配布用の bundle をまだ作っていない。実行ファイルの大きさが配布の大きさの目安になる。
※4 fcitx5 などの実機の IME が要る。Xvfb では測れない。

### 各回の生の値

`scripts/desktop-bench.sh -n 5` の出力 (poll_ms は絵を撮る間隔の平均)。

```
gpui run=1 first_paint_ms=230 poll_ms=86 rss_mib=197 rss_main_mib=197 pss_mib=194 cpu_idle_pct=0.2 procs=1
gpui run=2 first_paint_ms=251 poll_ms=99 rss_mib=197 rss_main_mib=197 pss_mib=194 cpu_idle_pct=0.3 procs=1
gpui run=3 first_paint_ms=221 poll_ms=86 rss_mib=197 rss_main_mib=197 pss_mib=194 cpu_idle_pct=0.2 procs=1
gpui run=4 first_paint_ms=241 poll_ms=85 rss_mib=197 rss_main_mib=197 pss_mib=194 cpu_idle_pct=0.2 procs=1
gpui run=5 first_paint_ms=214 poll_ms=97 rss_mib=196 rss_main_mib=196 pss_mib=194 cpu_idle_pct=0.2 procs=1
tauri-leptos run=1 first_paint_ms=381 poll_ms=84 rss_mib=336 rss_main_mib=153 pss_mib=192 cpu_idle_pct=0.0 procs=3
tauri-leptos run=2 first_paint_ms=253 poll_ms=85 rss_mib=335 rss_main_mib=152 pss_mib=192 cpu_idle_pct=0.0 procs=3
tauri-leptos run=3 first_paint_ms=182 poll_ms=86 rss_mib=335 rss_main_mib=152 pss_mib=192 cpu_idle_pct=0.0 procs=3
tauri-leptos run=4 first_paint_ms=191 poll_ms=85 rss_mib=335 rss_main_mib=152 pss_mib=192 cpu_idle_pct=0.0 procs=3
tauri-leptos run=5 first_paint_ms=176 poll_ms=84 rss_mib=335 rss_main_mib=152 pss_mib=192 cpu_idle_pct=0.1 procs=3
tauri-remote run=1 first_paint_ms=243 poll_ms=83 rss_mib=337 rss_main_mib=154 pss_mib=194 cpu_idle_pct=0.0 procs=3
tauri-remote run=2 first_paint_ms=229 poll_ms=83 rss_mib=336 rss_main_mib=153 pss_mib=193 cpu_idle_pct=0.0 procs=3
tauri-remote run=3 first_paint_ms=215 poll_ms=87 rss_mib=336 rss_main_mib=153 pss_mib=193 cpu_idle_pct=0.0 procs=3
tauri-remote run=4 first_paint_ms=218 poll_ms=84 rss_mib=336 rss_main_mib=153 pss_mib=193 cpu_idle_pct=0.0 procs=3
tauri-remote run=5 first_paint_ms=226 poll_ms=83 rss_mib=336 rss_main_mib=153 pss_mib=193 cpu_idle_pct=0.1 procs=3
```

実行ファイルの大きさ (byte): GPUI 60,994,808、Tauri 20,178,856。

### 読み取れること

- 起動時間と CPU は、この条件では差がない。差は測る間隔 (約 85 ms) と、回ごとのぶれ (Tauri (a) は 176〜381 ms) の範囲に収まる。
- メモリは見方で変わる。合計の RSS では GPUI が小さい (197 MiB 対 335 MiB) が、これは Tauri が WebKit の子プロセス (WebKitWebProcess など) を別に持ち、共有ライブラリのページを重複して数えるため。**本体だけなら GPUI が大きい** (197 MiB 対 152 MiB)。共有ページを按分した Pss の合計では、3 つともほぼ同じ (192〜194 MiB)。メモリの大きさで GPUI を選ぶ理由は、この測りからは出ない。GPUI の値には、フォントの 22 MiB と lavapipe の描画 (CPU) のぶんが入っていて、実機の GPU では変わる。
- 配布の大きさは GPUI が大きい。日本語フォントの同梱が約 22 MiB を占める。Tauri は OS のフォントと WebKitGTK を使う。
- GPUI で足りないのは、スクリーンリーダーと実機の IME の確認。残りの画面を GPUI で作り続けるかの判断では、この 2 つが重い。

## 常駐と通知の挙動 (GPUI 版)

- 通知: Notifications の Shape を常に購読し、`zoo2_client_core::notify::notifications_to_show` で判定する (Tauri 版と同じ関数)。DND は起動時と 5 分ごとに `GET /api/users/me/profile` の `dnd_until` を取る。取れなかったときは前の値を保ち、最初に取れなかったときは DND なしとして通知を出す。
- scope: `dnd_until` を返さないのは App のトークンで `preferences:read` が無いときで、OIDC のユーザートークンの既定 scope (`openid profile offline_access`) では要らない。scope は変えていない。
- 閉じたとき: gpui の Linux は最後のウィンドウを閉じるとイベントループが止まる。X11 は閉じる操作を止めて最小化し、トレイや 2 つ目の起動で `activate_window` する。Wayland は最小化から戻す手段がないので、閉じたら終了する。
- 単一インスタンス: ロックファイル (`flock`) が取れたものが最初のインスタンスで、残ったソケットは無条件に消して作り直す。取れなかったものは、最初のインスタンスのソケットにつないで「前に出す」を送って終わる (同時に起動しても、最初になるのは 1 つ)。ファイル名には設定ディレクトリの識別子を入れ、0600 にする。

動作の確認は `scripts/desktop-resident-smoke.sh` (Xvfb + openbox。X11 で、起動、2 つ目の起動、閉じる操作での最小化、2 つ目の起動での復帰、古いソケットの作り直し (inode が変わること)、同時起動を確かめる)。

## 制約 (GPUI 版の常駐)

- **Wayland**: 2 つ目の起動や通知のクリックでウィンドウを前に出せない。前に出すには xdg-activation のトークンが要るが、起動した側から既存のインスタンスへ渡す仕組みを入れていない。gpui の Wayland には最小化から戻す手段もないので、閉じたら終了する。
- **GNOME (X11)**: フォーカス奪取防止 (focus stealing prevention) のため、`activate_window` を呼んでも前に出ず、タスクバーの強調だけになることがある。
- **最小化に対応しない WM** (i3 など): 閉じる操作を止めて最小化を試みるので、最小化されず、閉じても何も起きない。トレイの「終了」で終わらせる。
- 通知のクリックで投稿まで移れるのは、その投稿が Store にあるとき (取得済み) だけ。まだ無いときは、チャンネルを開くだけになる。
- 通知のクリックを待つのは同時に 16 件まで。超えた分は、通知は出るがクリックでは何も起きない。

## 再現

```bash
nix develop -c bash -c 'cd desktop && cargo build --release'
nix develop -c scripts/desktop-bench.sh -n 5 gpui desktop/target/release/zoo2-desktop run
nix develop -c bash -c 'cd desktop-tauri/ui && trunk build --release && cd ../src-tauri && cargo build --release'
ZOO_DESKTOP_UI=leptos nix develop -c scripts/desktop-bench.sh -n 5 tauri-leptos desktop-tauri/src-tauri/target/release/zoo2-desktop-tauri
ZOO_DESKTOP_UI=remote nix develop -c scripts/desktop-bench.sh -n 5 tauri-remote desktop-tauri/src-tauri/target/release/zoo2-desktop-tauri
```
