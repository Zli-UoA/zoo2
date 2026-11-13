# デスクトップ版（Tauri）の設計

macOS と Linux 向けのデスクトップ版を Tauri v2 で作る。PR の CI（`ci-desktop-tauri.yaml`）は、Linux に加えて macOS（macos-15）でもビルドを確かめる。画面は 2 案を作って比べる検証の段階にある。

利用者向けの説明（インストール、ログイン、更新、困ったとき）は [desktop-tauri/README.md](../desktop-tauri/README.md) にある。

- (a) Leptos（CSR）で書いた画面を同梱する
- (b) `https://zli.chat` の React をリモートで読み込む

GPUI 版（`desktop/`）は比較の基準として残す。

| 項目 | 値 |
|---|---|
| ディレクトリ | `desktop-tauri/`（Rust 側は `desktop-tauri/src-tauri`、Leptos は `desktop-tauri/ui`） |
| crate 名 | `zoo2-desktop-tauri` |
| 更新タグ | `desktop-tauri-v*` |
| 設定ディレクトリ | `config_dir_for("zoo2-desktop-tauri")`（GPUI 版の `zoo2-desktop`、TUI の `zoo2-tui` とは別） |
| OIDC の client_id | `MwrfTPwsaaelMmxBdQGjWYuywWrsMHMe`（AuthZ の「Zoo for Desktop」。AuthZ が発行した ID。public client で、デバイスコードと refresh_token を許可し、resource `https://zli.chat/api` を紐付けてある） |

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
  - `frontend/src/components/auth/AuthGate.tsx`：Tauri の中では OIDC を通さず、`setTokenProvider` に `zoo_access_token` を渡す（`frontend/src/lib/desktop.ts`）。`expires_at` の 60 秒前まではキャッシュする。401 とログアウトでは `zoo_logout` を呼ぶ。
  - 通知のコレクション：初回の読み込みが終わった時点で、全行を `reset: true, up_to_date: true` のバッチで `zoo_notifications_observed` に渡す。それ以降は、変化ごとに `reset: false` のバッチで渡す。Rust 側はこれを (a) の通知の Shape と同じ流れで扱う。
  - 表示中のチャンネルとフォーカスを `zoo_set_focus` で、`zoo://open` を受けたらその画面へ移る。
- Tauri の中で動いているかは `window.__TAURI_INTERNALS__` の有無で見分ける。さらに `zoo_session` の `protocol` が `PROTOCOL_VERSION` と一致する場合だけ IPC を使う。Web は Tauri 版と別にデプロイされるので、一致しないことがある。一致しないときと `zoo_session` が失敗したときは、OIDC のログインへは進まず（Tauri の中では AuthZ への移動を `on_navigation` が止めて既定のブラウザーで開くため、ログインを完了できない）、アプリの更新を案内する画面と再読み込みのボタンを出す。
- capabilities：コマンドは `build.rs` の `AppManifest::commands` で 1 つずつ宣言する。宣言すると `allow-<名前>` の許可が作られ、capabilities でウィンドウごとに許せる。app の manifest があると、Tauri（2.12）は宣言のないコマンドを ACL がないものとして、ハンドラーに渡す前に拒否する。
  - `main-remote`：リモートの URL（`remote.urls`）は `Config.api_base_url` のオリジンだけにする。stg と `http://localhost:5173` は debug ビルドのときだけ足す。呼べるコマンドは `zoo_session`、`zoo_start_login`、`zoo_cancel_login`、`zoo_logout`、`zoo_set_focus`、`zoo_access_token`、`zoo_notifications_observed` と、`zoo://session` と `zoo://open` を受け取るための `core:event:allow-listen`、`core:event:allow-unlisten` に限る。
  - コマンドの中でも、呼び出し元のウィンドウの `UiMode` と URL のオリジンを確かめ、合わなければ `ErrorCode::WrongMode` を返す。
  - 許可したオリジン以外への移動は `on_navigation` で止め、既定のブラウザーで開く。
- Tauri の中では Web Push を使わない（`supportsPush()` が false を返す）。使うと、Rust 側の通知と二重に出る。Service Worker はオフライン保存に使うので止めない。
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

- ウィンドウを閉じたら隠し、トレイ（`tauri` の tray）に残す。トレイのメニューは「開く」と「終了」にする。更新の準備ができたら「更新して再起動」を足す（「配布と更新」）。
- 2 つ目の起動は `tauri-plugin-single-instance` で 1 つ目に渡し、ウィンドウを表示する。Wayland では前面に出ないことがある。
- ディープリンク（`tauri-plugin-deep-link`）は後続の項目にする。scheme が決まっておらず、Linux ではインストールした後でないと動かないため。
- 常駐と単一インスタンスは同じ段階で入れる。どちらかが動かない環境では、常駐をやめて「閉じたら終了」にする。

## 配布と更新

- **配布**: `cd-desktop-tauri-release.yaml` が、`desktop-tauri/src-tauri/Cargo.toml` の `version` を上げて `main` に入れたときに、`desktop-tauri-v<version>` の GitHub Release を作る（TUI の `cd-tui-release.yaml` と同じ流れ）。Linux は deb と AppImage を、glibc と WebKitGTK を古めに合わせるため ubuntu-22.04 で作る。macOS は Apple シリコンと Intel の両方で動く universal の `.app`（自動更新用の `.app.tar.gz`）と `.dmg` を macos-15 で作る。自動更新に使うものには、それぞれの署名（`.sig`）を置く。リポジトリの「Latest」は TUI のままにする（`--latest=false`）。
- **Nix**: flake の `packages.<system>.zoo2-desktop-tauri`（Linux のみ）。同梱の画面を `trunk build --offline` で先に作り（`zoo2-desktop-tauri-ui`）、`tauri/custom-protocol` の feature で Rust 側に埋め込む。`wrapGAppsHook3` で GIO のモジュールと schema を渡し、dlopen で読まれる `libayatana-appindicator` は `LD_LIBRARY_PATH` に足す。Nix のストアは書き換えられず、`bundle_type()` も取れないので、アプリの中からは更新しない。更新は flake の入力を上げて行う。
- **更新の確認**: `tauri-plugin-updater` を Rust 側だけで使う（WebView には許可を出さない）。起動の 30 秒後と 6 時間ごとに `latest.json` を読み、新しい版があれば裏で取得して署名を確かめる。入れ替えと再起動は、通知かトレイのメニューを利用者が押したときだけ行う（書きかけの下書きを勝手に失わないため）。`check_updates = false`（`ZOO_CHECK_UPDATES=false`、TUI と同じ設定）なら確かめない。
- **`latest.json` の置き場所**: 「Latest」が TUI なので `releases/latest/download/` は使えない。決まったタグ `desktop-tauri-updater` のリリース（prerelease）を 1 つ作り、リリースのたびに `latest.json` だけを置き直す。`platforms` には `linux-x86_64-deb` と `linux-x86_64-appimage`（と、古い形の `linux-x86_64`）、`darwin-aarch64` と `darwin-x86_64`（と `-app` 付き。どちらも universal の `.app.tar.gz`）を書く。署名がない形は書かない。プラグインは、ビルドしたときの形（deb なら `dpkg -i` を `pkexec` で、AppImage ならファイルの置き換え）で入れる。リリース済みの版でも、`latest.json` がその版を指していなければ、workflow は既存のリリースの署名から作り直して公開する（公開が途中で失敗しても、再実行で直る）。
- **署名の鍵**: 秘密鍵は Secrets の `TAURI_SIGNING_PRIVATE_KEY` と `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`、公開鍵は Variables の `TAURI_UPDATER_PUBKEY` に置く。公開鍵はリリースのビルドのときだけ `--config` で `plugins.updater.pubkey` に入れる。リポジトリの `tauri.conf.json` は空のままにして、ローカルや CI のビルド、`cargo tauri dev` では更新を確かめない（`bundle_type()` が取れないときも確かめない）。鍵がなければ、workflow はリリースを作らずに終わる。
- **鍵の作り方と保管**: 管理者が手元で作る。秘密鍵とパスワードは、zoo2-infra の `github/zoo2-desktop-tauri/updater-signing-key.sops.yaml` に sops で暗号化して置き、これを正本にする（GitHub Secrets の値は後から読み出せないため）。GitHub Secrets へは、そこから `sops decrypt --extract` の出力を `gh secret set` に渡して登録する。手順は zoo2-infra の `github/README.md` にある。秘密鍵をなくすと、配った版はそれ以降の更新を受け付けない。
- **後続の項目**:
  - `requireSignedVersion`（署名に版を含め、古い版への差し戻しを防ぐ）は入れていない。いまの tauri-cli（2.11.4）は署名に版を書かないため。CLI を上げたら有効にする。
  - macOS は、いまは ad-hoc 署名（`signingIdentity: "-"`）だけで、Developer ID の署名と公証をしていない。利用者は初回に quarantine を外すか「このまま開く」を押す。Apple Developer Program に入ったら、証明書と公証の Secrets を足して有効にする。

## 未確定・確認が要るもの

- AuthZ が「Zoo for Desktop」に refresh token を発行するか。`frontend/.env.example` には「offline_access は非対応で、refresh token は発行されません」と書かれている。TUI の `TokenManager` は refresh token がなくても動き、期限が切れたらログインし直しになる。検証の最初に TUI で確かめる。
- ループバックの redirect（RFC 8252）を AuthZ が受け付けるか。今回はデバイスコードを使うので依存しないが、要確認として残す。

## 開発

`nix develop` の中で動かす。Tauri の CLI、trunk、wasm-bindgen、WebKitGTK が入っている。

```sh
cd desktop-tauri/src-tauri
cargo tauri dev                    # trunk serve (localhost:1420) と一緒に起動する
cargo tauri build --bundles deb    # NixOS では AppImage を作れないので deb だけ。AppImage は CI で作る
```

- 画面の案は `desktop_ui`（`ZOO_DESKTOP_UI`）の `remote|leptos` で選ぶ。既定は `remote`（Web 版の画面。通話もこちらでだけ使える）。ただし `ZOO_DEV_USER` を付けたとき（dev 認証）は `leptos` にする。ローカルの backend は Web の画面を配信しないため。`desktop_ui` は `client-core` の `desktop_ui_for` で Tauri 版だけが読み、`Config` には入れない（値を誤っても TUI や GPUI 版の起動を止めないように）。知らない値は設定のエラーとして同梱の画面に出す（打ち間違いに気づけるように）。TUI と GPUI 版は読まないので影響しない。
- macOS の署名は ad-hoc でも Hardened Runtime になるので、マイクとカメラの entitlement（`src-tauri/Entitlements.plist`、`bundle.macOS.entitlements`）が要る。CI の macOS のジョブで、署名したアプリに入っていることを `codesign` で確かめる。
- 通話: (b) の画面にだけ、マイクとカメラの許可を与える（`on_permission_request`）。macOS は `src-tauri/Info.plist` の `NSMicrophoneUsageDescription` と `NSCameraUsageDescription` で OS が使用を尋ねる。Linux は WebKitGTK の WebRTC と media stream を既定で無効にしているので、ウィンドウを作ったあとに有効にする。
- macOS の通知は、notify-rust の `preview-macos-un` で UNUserNotificationCenter から出し、起動時に許可を求める（古い NSUserNotification は許可を求められず、Finder として出そうとして表示されないことがある）。
- (b) で読み込む先は `api_base_url` の起点。debug ビルドだけ、`ZOO_DESKTOP_REMOTE_URL=http://localhost:5173` で Vite の dev サーバーに差し替えられる（React 側の変更をデプロイ前に試すため）。
- 接続先とログインは TUI と同じ環境変数（`ZOO_API_BASE_URL`、`ZOO_OIDC_CLIENT_ID`、`ZOO_OIDC_RESOURCE` など）か、`~/.config/zoo2-desktop-tauri/config.toml` で変える。
- ローカルの backend（`DEV_AUTH_ENABLED=true`）に向けるときは `ZOO_DEV_USER=<UUID>` を付ける。デバイスコードのログインを通らずに `X-Dev-User` で接続する。画面は同梱の `leptos` になる。Web の画面を試すときは、frontend の dev サーバー（`pnpm dev`）を立てて、debug ビルドで `ZOO_DESKTOP_UI=remote ZOO_DESKTOP_REMOTE_URL=http://localhost:5173` を付ける。
- NVIDIA と Wayland の組み合わせで画面が出ないときは、`WEBKIT_DISABLE_DMABUF_RENDERER=1` を付ける。
- `wasm-bindgen` は `desktop-tauri/ui/Cargo.toml` で、nixpkgs の `wasm-bindgen-cli` と同じ版に固定している。flake.lock を更新して版が変わったら、両方をそろえる。
