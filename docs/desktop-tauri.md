# デスクトップ版（Tauri）の設計

macOS と Linux 向けのデスクトップ版を Tauri v2 で作る。画面は 2 案を作って比べる検証の段階にある。

- (a) Leptos（CSR）で書いた画面を同梱する
- (b) `https://zli.chat` の React をリモートで読み込む

GPUI 版（`desktop/`）は比較の基準として残す。

| 項目 | 値 |
|---|---|
| ディレクトリ | `desktop-tauri/`（Rust 側は `desktop-tauri/src-tauri`、Leptos は `desktop-tauri/ui`） |
| crate 名 | `zoo2-desktop-tauri` |
| 更新タグ | `desktop-tauri-v*` |
| 設定ディレクトリ | `config_dir_for("zoo2-desktop-tauri")`（GPUI 版の `zoo2-desktop`、TUI の `zoo2-tui` とは別） |
| OIDC の client_id | `zoo2-desktop-tauri`（AuthZ に public client として登録する。デバイスコードと refresh_token を許可する） |

## crate の層

Cargo workspace にはまとめず、path 依存でつなぐ。

```
client-types/   zoo2-client-types   型と純粋関数。wasm32 でもビルドできる（通信・ファイル・時計に触れない）
                  rows    Shape の行の型（Channel、User、Message、Notification など）
                  shape   Row、Batch、Change、ShapeId、Replica
                  store   行を保持して画面に出す形を作る
                  mfm     MFM のパーサー
                  notify  OS の通知を出すかの判定
                  api     REST の応答のうち両側で使うもの（Me）
                  ipc     Tauri のコマンド・イベントの名前と型
client-core/    zoo2-client-core    ネイティブ専用。通信（ShapeStream、ApiClient）、認証（デバイスコード、TokenManager）、設定、更新の確認
                                    types のモジュールを同じパスで再公開しているので、TUI と GPUI 版の import は変わらない
desktop-tauri/src-tauri             Tauri のコマンドとイベントの層
desktop-tauri/ui                    (a) Leptos の画面（wasm32）。client-types だけに依存する
```

## 同期の境界

Electric の Shape を購読する側は、(a) と (b) で異なる。**どちらの案でも、同じ Shape を Rust 側と WebView 側の両方で購読しない。**

| | (a) Leptos | (b) React |
|---|---|---|
| Shape の購読 | Rust 側（`client-core::electric::spawn`） | WebView 側（既存の TanStack DB / Electric） |
| 行の保持 | Rust 側が `Replica`（生の行）と `Store` を持つ。WebView 側も `Store` の写しを持つ | WebView 側だけ |
| 書き込み（REST） | `zoo_send_message` で Rust 側が呼ぶ | WebView 側が直接呼ぶ |
| アクセストークン | Rust 側だけが持つ | `zoo_access_token` で WebView に渡す |
| 通知の Shape | Rust 側が購読する | React が購読している（通知の画面で使う）。React は変化を `zoo_notifications_observed` で Rust 側に渡す |
| 通知の判定と表示 | Rust 側（`client_types::notify`、`notify-rust`） | 同じ |
| ログイン画面 | Leptos | Leptos の同じ画面（下記） |

### (a) Leptos

```
Rust 側                                                  WebView 側 (Leptos)
TokenManager ─ Credentials ─┐
                            ├ spawn(Channels / Users / ChannelUsers / Notifications)  常に購読
                            └ spawn(Messages(開いているチャンネル))                    zoo_open_channel で切り替え
ShapeEvent ─▶ 1 本のループ
   1. Notifications なら notify::notifications_to_show(&store, &batch, &ctx)  ← 適用より前に判定する
   2. replica.apply / store.apply
   3. emit("zoo://shape", ShapeBatch)  ───────────────────────────────▶  store.apply（写し）
                                                                          起動・再読み込みの直後:
                                       ◀──────────────── invoke("zoo_session") → invoke("zoo_resync")
   replica.snapshot() を reset 付きで emit  ──────────────────────────▶  store を作り直す
```

- イベントは 1 本のループから順に送るので、WebView 側に届く順番は Shape に届いた順と同じになる。
- `zoo_resync` で送る各バッチは `reset: true` なので、何度受け取っても WebView 側の状態は同じになる。読み込み直しで行が抜けたり二重になったりしない。
- チャンネルを切り替えると、WebView 側は前のチャンネルの行を `Store::forget` で捨ててから `zoo_open_channel` を呼ぶ。Rust 側は前のチャンネルの購読タスクを止め、`Replica::forget` する。
- 書き込みの結果は Shape で届くので、検証の段階では楽観的な表示はしない。`SendResult::txid` は、後で楽観的な表示を入れるときに使う。

### (b) React（リモート）

- 起動したとき、有効なトークンがなければ、同梱の Leptos のログイン画面（(a) と同じもの）を表示する。`Ready` になったら、ウィンドウを `https://zli.chat` に移す。
- React からログアウトしたら、Rust 側がトークンを消し、ウィンドウを同梱のログイン画面に戻す。
- React 側に足す差し替え口は次のとおり。
  - `frontend/src/lib/electric/headers.ts` と `frontend/src/lib/api/client.ts`：Tauri の中では、OIDC の代わりに `zoo_access_token` からトークンを取る。`expires_at` の 60 秒前まではキャッシュする。
  - 通知のコレクション：初回の読み込みが終わった時点で、全行を `reset: true, up_to_date: true` のバッチで `zoo_notifications_observed` に渡す。それ以降は、変化ごとに `reset: false` のバッチで渡す。Rust 側はこれを (a) の通知の Shape と同じ流れで扱う。
  - 表示中のチャンネルとフォーカスを `zoo_set_focus` で、`zoo://open` を受けたらその画面へ移る。
- Tauri の中で動いているかは `window.__TAURI_INTERNALS__` の有無で見分ける。さらに `zoo_session` の `protocol` が `PROTOCOL_VERSION` と一致する場合だけ IPC を使う。一致しなければ通常の Web として動く（Web は Tauri 版と別にデプロイされるため）。
- capabilities：コマンドは `build.rs` の `AppManifest::commands` で 1 つずつ宣言する。宣言しないと許可の識別子が作られず、全ウィンドウから呼べてしまう。
  - `main-remote`：リモートの URL（`remote.urls`）は `Config.api_base_url` のオリジンだけにする。stg と `http://localhost:5173` は debug ビルドのときだけ足す。呼べるコマンドは `zoo_session`、`zoo_start_login`、`zoo_cancel_login`、`zoo_logout`、`zoo_set_focus`、`zoo_access_token`、`zoo_notifications_observed` と、`zoo://session` と `zoo://open` を受け取るための `core:event:allow-listen`、`core:event:allow-unlisten` に限る。
  - コマンドの中でも、呼び出し元のウィンドウの `UiMode` と URL のオリジンを確かめ、合わなければ `ErrorCode::WrongMode` を返す。
  - 許可したオリジン以外への移動は `on_navigation` で止め、既定のブラウザーで開く。
- Tauri の中では、Service Worker の登録と Web Push を止める。止めないと、Rust 側の通知と二重に出る。
- (a) の同梱の画面からは、`zoo_access_token` と `zoo_notifications_observed` を呼べなくする。トークンを WebView に出さないため。

## 通知

判定は `client_types::notify::notifications_to_show` で、テストのある純粋関数にしてある。条件は次のとおり。

- 初回の同期が終わってから届いた未読だけを出す。初回の同期と `reset` のバッチは出さない（起動や再接続のたびに過去の通知が大量に出ないようにする）。
- 自分が起こしたもの、ウィンドウが前面にあって表示中のチャンネルのものは出さない。
- DND（`GET /api/users/me/profile` の `dnd_until` が今より後）の間は出さない。止めた分を後でまとめて出すこともしない。backend の Web Push（`backend/src/push.rs`）と同じ扱い。`dnd_until` は起動時と 5 分ごとに取り直す。
- 届くのは、アプリが動いている間（トレイに常駐している間を含む）だけ。アプリを終了している間の通知（APNs や UnifiedPush）は範囲外とする。
- 表示は `notify-rust` で行う。`tauri-plugin-notification` はデスクトップでクリックを受け取れないため使わない。
- Linux では通知に action を付け、専用のスレッドで `wait_for_action` を待つ。クリックされたら、ウィンドウを前に出して `zoo://open` を送る。
- macOS では通知を出すだけで、クリックには対応しない（後続の項目）。

## 常駐、単一インスタンス

- ウィンドウを閉じたら隠し、トレイ（`tauri` の tray）に残す。トレイのメニューは「開く」と「終了」にする。
- 2 つ目の起動は `tauri-plugin-single-instance` で 1 つ目に渡し、ウィンドウを表示する。Wayland では前面に出ないことがある。
- ディープリンク（`tauri-plugin-deep-link`）は後続の項目にする。scheme が決まっておらず、Linux ではインストールした後でないと動かないため。
- 常駐と単一インスタンスは同じ段階で入れる。どちらかが動かない環境では、常駐をやめて「閉じたら終了」にする。

## 未確定・確認が要るもの

- AuthZ が `zoo2-desktop-tauri` に refresh token を発行するか。`frontend/.env.example` には「offline_access は非対応で、refresh token は発行されません」と書かれている。TUI の `TokenManager` は refresh token がなくても動き、期限が切れたらログインし直しになる。検証の最初に TUI で確かめる。
- ループバックの redirect（RFC 8252）を AuthZ が受け付けるか。今回はデバイスコードを使うので依存しないが、要確認として残す。
