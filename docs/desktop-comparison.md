# デスクトップクライアントの比較

zoo2 のデスクトップクライアントは 3 通りある。GPUI 版 (`desktop/`)、Tauri (a) Leptos 版、Tauri (b) remote React 版 (`desktop-tauri/`。(a) と (b) は `ZOO_DESKTOP_UI=leptos|remote` で切り替える)。この文書は、残りの画面を GPUI で作り続けるかを決める材料として、同じ条件で測れたものと、測れなかったものを並べる。

測っていないものは「未計測」と書き、理由を添える。推測の数字は入れていない。

## 測り方

- 環境: Xvfb (1280x800) + openbox + lavapipe (CPU の Vulkan)。`nix develop` の中で `scripts/desktop-bench.sh` を動かした。実機の GPU ではない。
- 対象の画面: **ログイン前の画面**。backend に接続せず、AuthZ のログインもしていない (`ZOO_API_BASE_URL=http://127.0.0.1:9`)。
- ビルド: GPUI は `cargo build --release` (`strip = true`)、Tauri は `trunk build --release` と `cargo build --release`。どれも release で、debug の値は使っていない。
- 回数: 各 3 回 (表は中央値、起動時間は範囲も書く)。
- 起動時間: 起動から、root ウィンドウの絵が空でなくなるまでを 50 ms 刻みで確かめた値。ウィンドウ 1 枚目の描画の目安であり、フレーム単位の計測ではない。1 回目は OS のファイルキャッシュが冷えているぶん遅い。
- RSS: 描画の 5 秒後の、プロセスと子プロセス全体の VmRSS の合計。共有ライブラリのページは重複して数えるので、実メモリより多い。
- CPU: 描画 5 秒後から 10 秒間の、プロセス全体の使用率 (1 コア = 100%)。

## 比較表

| 項目 | GPUI | Tauri (a) Leptos | Tauri (b) remote React |
|---|---|---|---|
| 最初の描画まで (中央値、範囲) | 544 ms (532〜1,375 ms) | 563 ms (384〜748 ms) | 551 ms (401〜563 ms) ※1 |
| RSS (合計 / 本体) | 196 MiB / 196 MiB (1 プロセス) | 332 MiB / 150 MiB (3 プロセス) | 334 MiB / 152 MiB (3 プロセス) ※1 |
| CPU アイドル時 | 0.2% | 0.0% | 0.1% ※1 |
| CPU スクロール時 | 未計測 ※2 | 未計測 ※2 | 未計測 ※2 |
| 実行ファイル | 58.1 MiB (release、strip 済み。Noto Sans JP 4 ウェイト約 22 MiB を含む) | 19.2 MiB (`target/release`。CI の deb 内は 19.5 MiB) | (a) と同じ実行ファイル |
| 配布物 (bundle) | 未作成 ※3 | deb 7.7 MiB、AppImage 83.3 MiB (CI の成果物) | (a) と同じ |
| 実行時の依存 | なし (X11 / Wayland、Vulkan) | WebKitGTK 4.1 | WebKitGTK 4.1 + 配信元の frontend |
| IME | 未計測 ※4 | 未計測 ※4 (WebKitGTK の IME) | 未計測 ※4 (WebKitGTK の IME) |
| 通知 | notify-rust (D-Bus)。Linux はクリックで前面に出し、そのチャンネルを開く。macOS は出すだけ | notify-rust。同じ判定 (`client-types::notify`) | 同左 |
| トレイ | Linux は ksni、macOS は tray-icon (macOS は未検証)。メニューは「開く」「終了」 | Tauri のトレイ。メニューは「開く」「終了」 | 同左 |
| 閉じたときの挙動 | X11 は最小化してトレイに常駐。Wayland は終了 (戻す手段がない) | トレイに常駐 | 同左 |
| 単一インスタンス | Unix ソケット (`$XDG_RUNTIME_DIR/zoo2-desktop.sock`) | tauri-plugin-single-instance | 同左 |
| スクリーンリーダー | 非対応 (gpui 0.2.2 は AccessKit などの対応がない) | WebKitGTK の AT-SPI (未検証) | 同左 (未検証) |
| 日本語フォント | Noto Sans JP 4 ウェイトを同梱 (約 22 MiB)。Geist は静的な 4 ウェイトを同梱 | OS のフォント | OS のフォント (frontend が指定する Web フォント) |

※1 (b) は backend (frontend の配信元) に届かない状態で測ったので、WebKit のエラーページを描いている。React の読み込みと描画は含まない。(b) の数字は参考にならず、実際の起動時間と RSS は未計測 (backend と AuthZ が要る)。
※2 スクロールの CPU は、ログインしてメッセージの入った backend につなぐ必要がある。AuthZ のログインと backend を使わない条件で測ったので、計測していない。
※3 GPUI 版は配布用の bundle をまだ作っていない。実行ファイルの大きさが配布の大きさの目安になる。
※4 fcitx5 などの実機の IME が要る。Xvfb では測れない。

### 読み取れること

- 起動時間と CPU は、この条件では差が小さい。差は 1 回目の冷えたキャッシュのぶれより小さい。
- RSS は GPUI が小さい (196 MiB 対 332 MiB)。ただし GPUI はフォント 22 MiB と lavapipe の描画 (CPU) のぶんを含む。実機の GPU では変わる。
- 配布の大きさは GPUI が大きい。日本語フォントの同梱が約 22 MiB を占める。Tauri は OS のフォントと WebKitGTK を使う。
- GPUI で足りないのは、スクリーンリーダーと実機の IME の確認。残りの画面を GPUI で作り続けるかの判断では、この 2 つが重い。

## 常駐と通知の挙動 (GPUI 版)

- 通知: Notifications の Shape を常に購読し、`zoo2_client_core::notify::notifications_to_show` で判定する (Tauri 版と同じ関数)。DND は起動時と 5 分ごとに `GET /api/users/me/profile` の `dnd_until` を取る。取れなかったときは前の値を保ち、最初に取れなかったときは DND なしとして通知を出す。
- scope: `dnd_until` を返さないのは App のトークンで `preferences:read` が無いときで、OIDC のユーザートークンの既定 scope (`openid profile offline_access`) では要らない。scope は変えていない。
- 閉じたとき: gpui の Linux は最後のウィンドウを閉じるとイベントループが止まる。X11 は閉じる操作を止めて最小化し、トレイや 2 つ目の起動で `activate_window` する。Wayland は最小化から戻す手段がないので、閉じたら終了する。
- 単一インスタンス: ソケットに接続できなければ、残ったソケットを消して作り直す。

動作の確認は `scripts/desktop-resident-smoke.sh` (Xvfb + openbox。X11 で、起動、2 つ目の起動、閉じる操作での最小化、2 つ目の起動での復帰、古いソケットの作り直しを確かめる)。

## 再現

```bash
nix develop -c bash -c 'cd desktop && cargo build --release'
nix develop -c scripts/desktop-bench.sh gpui desktop/target/release/zoo2-desktop run
nix develop -c bash -c 'cd desktop-tauri/ui && trunk build --release && cd ../src-tauri && cargo build --release'
ZOO_DESKTOP_UI=leptos nix develop -c scripts/desktop-bench.sh tauri-leptos desktop-tauri/src-tauri/target/release/zoo2-desktop-tauri
ZOO_DESKTOP_UI=remote nix develop -c scripts/desktop-bench.sh tauri-remote desktop-tauri/src-tauri/target/release/zoo2-desktop-tauri
```
