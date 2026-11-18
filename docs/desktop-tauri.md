# デスクトップ版（Tauri）の設計

Linux・macOS・Windows 向けのデスクトップ版を Tauri v2 で作る。PR の CI（`ci-desktop-tauri.yaml`）は、Linux に加えて macOS（macos-15）と Windows（windows-latest）でもビルドを確かめる。画面は 2 案を作って比べる検証の段階にある。

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
| 通話の招待の Shape | Rust 側が購読する（着信のトースト用） | Rust 側が購読する（着信のトースト用。画面の表示用には、WebView 側も別に購読する）。詳しくは [音声通話と呼び出し](calls.md) |
| 通知の判定と表示 | Rust 側（`client_types::notify`、Unix は `notify-rust`、Windows は WinRT `ToastNotificationManager`） | 同じ |
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
  - `main-remote`：リモートの URL（`remote.urls`）は `Config.api_base_url` のオリジンだけにする。stg と `http://localhost:5173` は debug ビルドのときだけ足す。呼べるコマンドは `zoo_session`、`zoo_start_login`、`zoo_cancel_login`、`zoo_logout`、`zoo_set_focus`、`zoo_set_badge`、`zoo_access_token`、`zoo_notifications_observed`、`zoo_take_call_accept`、`zoo_reauthenticate` と、`zoo://session`、`zoo://open`、`zoo://call-accept` を受け取るための `core:event:allow-listen`、`core:event:allow-unlisten` に限る。
  - コマンドの中でも、呼び出し元のウィンドウの `UiMode` と URL のオリジンを確かめ、合わなければ `ErrorCode::WrongMode` を返す。
  - 許可したオリジン以外への移動は `on_navigation` で止め、既定のブラウザーで開く。
- E2EE の再認証：Web の `startReauth` は、Tauri の中では別のウィンドウを開かず、`zoo_reauthenticate` を呼ぶ。Rust 側は `client-core` の `auth/reauth.rs` で、既定のブラウザーで `max_age=0` の認可リクエストを開き、ループバックのリダイレクト（`http://127.0.0.1:<空いているポート>/callback`）で受け取った ID トークンを返す。wry は新しいウィンドウのハンドラーがないとウィンドウを作らず、アプリの WebView ではパスキー（AuthZ が再認証として認める唯一のログイン）も使えないため（`docs/e2ee.md` のクロス署名の API の【決定】）。受け取ったらメインのウィンドウを前に出す。新しく始めたら、待っている前の再認証は取り消す（`ErrorCode::Cancelled`）。時間切れは `ErrorCode::Timeout`、AuthZ の拒否と ID トークンの不一致は `ErrorCode::Rejected` で返し、Web は `ReauthError` に変える。
- Tauri の中では Web Push を使わない（`supportsPush()` が false を返す）。使うと、Rust 側の通知と二重に出る。Service Worker はオフライン保存に使うので止めない。
- (a) の同梱の画面からは、`zoo_access_token` と `zoo_notifications_observed` を呼べなくする。トークンを WebView に出さないため。

## 通知

判定は `client_types::notify::notifications_to_show` で、テストのある純粋関数にしてある。条件は次のとおり。

- 初回の同期が終わってから届いた未読だけを出す。初回の同期と `reset` のバッチは出さない（起動や再接続のたびに過去の通知が大量に出ないようにする）。
- 自分が起こしたもの、ウィンドウが前面にあって表示中のチャンネルのものは出さない。
- DND（`GET /api/users/me/profile` の `dnd_until` が今より後）の間は出さない。止めた分を後でまとめて出すこともしない。backend の Web Push（`backend/src/push.rs`）と同じ扱い。`dnd_until` は起動時と 5 分ごとに取り直す。
- 届くのは、アプリが動いている間（トレイに常駐している間を含む）だけ。アプリを終了している間の通知（APNs や UnifiedPush）は範囲外とする。
- 表示は Linux・macOS では `notify-rust`、Windows では WinRT の `ToastNotificationManager`（`toast.rs`）で行う。`tauri-plugin-notification` はデスクトップでクリックを受け取れないため使わない。Windows では起動時に HKCU 配下へ AUMID（`chat.zli.desktop`）と通知用アイコンを登録し、通常通知のクリックと通話着信のボタン操作（「応答」「拒否」）を受け取る。
- Linux では通知に action を付け、専用のスレッドで `wait_for_action` を待つ。クリックされたら、ウィンドウを前に出して `zoo://open` を送る。
- macOS では通知を出すだけで、クリックには対応しない（後続の項目）。
- **未読バッジ**: React が `useUnifiedNotifications().unreadCount`（チャンネルと ActivityPub の未読から重複を引いた数）を `zoo_set_badge` で渡し、Rust 側が `set_badge_count` で Dock やランチャーのアイコンに出す。`0` 以下で消す。Rust 側のストアには ActivityPub の分がないので、数は React から受け取る。コマンドは同期にし、IPC の到着順は保証されないので、WebView 側が前の呼び出しの完了を待ってから次を送る（待つ間の値は最後の 1 つだけ送る）。ログアウトや期限切れ（`follow_session`）では Rust 側でも消す。macOS の Dock では表示される。Linux では、libunity が入っていて `com.canonical.Unity.LauncherEntry` を実装するドックが動いているときだけ出る（素の GNOME では出ない）。デスクトップファイル名が `zoo2.desktop`（`productName`）と一致することも要る。Windows は `set_badge_count` 非対応のため出ない。古い版のアプリにはこのコマンドがないので、Web 側は呼べなくても何もしない（`PROTOCOL_VERSION` は上げない）。PWA では、同じ未読数を `navigator.setAppBadge` で出す（`AppBadgeBridge`）。アプリを閉じている間の更新はしない（サーバーの push に未読数がないため）。

## 常駐、単一インスタンス

- ウィンドウを閉じたら隠し、トレイ（`tauri` の tray）に残す。トレイのメニューは「開く」と「終了」にする。更新の準備ができたら「更新して再起動」を足す（「配布と更新」）。
- 2 つ目の起動は `tauri-plugin-single-instance` で 1 つ目に渡し、ウィンドウを表示する。Wayland では前面に出ないことがある。
- ディープリンク（`tauri-plugin-deep-link`）は後続の項目にする。scheme が決まっておらず、Linux ではインストールした後でないと動かないため。
- 常駐と単一インスタンスは同じ段階で入れる。どちらかが動かない環境では、常駐をやめて「閉じたら終了」にする。

## 配布と更新

- **配布**: `cd-desktop-tauri-release.yaml` が、`desktop-tauri/src-tauri/Cargo.toml` の `version` を上げて `main` に入れたときに、`desktop-tauri-v<version>` の GitHub Release を作る（TUI の `cd-tui-release.yaml` と同じ流れ）。Linux は deb と AppImage を、glibc と WebKitGTK を古めに合わせるため ubuntu-22.04 で作る。macOS は Apple シリコンと Intel の両方で動く universal の `.app`（自動更新用の `.app.tar.gz`）と `.dmg` を macos-15 で作る。Windows は `windows-latest` 上で `cargo tauri build --no-bundle` と Inno Setup（`desktop-tauri/installer/zoo2.iss`）を組み合わせて `zoo2_<version>_x64-setup.exe` を作る（ユーザー権限・WebView2 未導入時はブートストラッパーから導入・アンインストール時に設定とログインデータの削除有無を確認）。自動更新に使うものには、それぞれの署名（`.sig`。Inno Setup 版は `cargo tauri signer sign` で作成）を置く。リポジトリの「Latest」は TUI のままにする（`--latest=false`）。
- **Nix**: flake の `packages.<system>.zoo2-desktop-tauri`（Linux のみ）。同梱の画面を `trunk build --offline` で先に作り（`zoo2-desktop-tauri-ui`）、`tauri/custom-protocol` の feature で Rust 側に埋め込む。`wrapGAppsHook3` で GIO のモジュールと schema を渡し、dlopen で読まれる `libayatana-appindicator` は `LD_LIBRARY_PATH` に足す。Nix のストアは書き換えられず、`bundle_type()` も取れないので、アプリの中からは更新しない。更新は flake の入力を上げて行う。
- **更新の確認**: `tauri-plugin-updater` を Rust 側だけで使う（WebView には許可を出さない）。起動の 30 秒後と 6 時間ごとに `latest.json` を読み、新しい版があれば裏で取得して署名を確かめる。Windows の `--no-bundle` + Inno Setup 版は `bundle_type()` が `None` になるため、レジストリの Inno Setup アンインストール情報（`InstallLocation`・`UninstallString`）が実行ファイル配置先の `uninsNNN.exe` の実在ファイルと一致すること（`is_inno_installed`）でインストール済みを判定する。自動更新時は `Update::verify` で Minisign 署名を検証したうえで `<app_local_data_dir>/updates/zoo2-updater-*.exe.part` を書き出し、システム PowerShell の隠しプロセスが `.lock` への排他ロック確立（`LOCKED` 通知）後に `.exe` へ改名して、アプリ終了（`WaitForExit`）・Inno Setup 実行（`/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /DIR="..." /WAITPID=<pid> /RELAUNCH=1`）を見守り、正常終了時は Inno Setup 側で再起動し、非 0 終了時は一時インストーラー削除後に旧版を `--update-failed <code>` 付きで再起動して `updater_window` にエラーを出す。入れ替えと再起動は、通知かトレイのメニューを利用者が押したときだけ行う（書きかけの下書きを勝手に失わないため）。`check_updates = false`（`ZOO_CHECK_UPDATES=false`、TUI と同じ設定）なら自動では確かめない。
- **メニューバーとトレイからの手動確認**: macOS はアプリメニュー（`window::create_app_menu`）、Linux と Windows はトレイメニュー（`window::create_tray`）に「アップデートを確認…」を足す。押すと `updater_window` を開き、準備済みの更新があればすぐ入れて再起動し、なければ「更新を確かめています」を出してその場で確かめ、新しい版があれば取得して入れて再起動する。新しい版がないときと失敗したときは OS 通知ではなく `updater_window` に結果を出し、少し後に閉じる。自動の確認と重ならないよう、確かめている間は「更新を確かめています」を出したまま進行中の確認へ結果表示を予約し、確認が終わったときに同じ `updater_window` へ結果を出す。利用者が自分で押したときだけ確かめるので、`check_updates = false` でも使える。確認とダウンロードの通信には、受信が 30 秒止まったら諦める `read_timeout` と、接続の `connect_timeout`（15 秒）を付ける（`configure_client`）。応答が止まっても、確かめている最中のまま残らないように。全体の時間には上限を付けない（遅い回線でも、受信が進んでいれば数十 MB の取得を終えられるように）。
- **更新のウィンドウ**: 利用者が更新を始めたとき（メニューの「アップデートを確認…」、トレイや通知の「更新して再起動」）は、アイコン、状態の文言、進捗バーだけの小さなウィンドウ（`updater_window`、画面は `ui/updater/`）に、確認、ダウンロードの進み具合（1% ごと）、インストールを順に出す。最新のときと失敗したときは結果を出して、少し後に閉じる。画面には IPC を許さず、状態は Rust 側から `window.zooUpdater.set()` を `eval` で渡す（読み込みの前に渡した状態は `on_page_load` で渡し直す）。自動の確認は裏で静かに行い、自動の取得の最中にメニューから押されたら、取得が終わり次第そのまま入れる。
- **`latest.json` の置き場所**: 「Latest」が TUI なので `releases/latest/download/` は使えない。決まったタグ `desktop-tauri-updater` のリリース（prerelease）を 1 つ作り、リリースのたびに `latest.json` だけを置き直す。`platforms` には `linux-x86_64-deb` と `linux-x86_64-appimage`（と、古い形の `linux-x86_64`）、`darwin-aarch64` と `darwin-x86_64`（と `-app` 付き。どちらも universal の `.app.tar.gz`）、`windows-x86_64`（Inno Setup の `zoo2_<version>_x64-setup.exe`）を書く。署名がない形は書かない。プラグインは、ビルドしたときの形（deb なら `dpkg -i` を `pkexec` で、AppImage ならファイルの置き換え、Windows Inno Setup 版は上記インストーラー起動）で入れる。リリース済みの版でも、`latest.json` がその版を指していなければ、workflow は既存のリリースの署名から作り直して公開する（公開が途中で失敗しても、再実行で直る）。
- **署名の鍵**: 秘密鍵は Secrets の `TAURI_SIGNING_PRIVATE_KEY` と `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`、公開鍵は Variables の `TAURI_UPDATER_PUBKEY` に置く。公開鍵はリリースのビルドのときだけ `--config` で `plugins.updater.pubkey` に入れる。リポジトリの `tauri.conf.json` は空のままにして、ローカルや CI のビルド、`cargo tauri dev` では更新を確かめない（Linux / macOS で `bundle_type()` が取れないときや、Windows の非インストール版でも確かめない）。鍵がなければ、workflow はリリースを作らずに終わる。
- **鍵の作り方と保管**: 管理者が手元で作る。秘密鍵とパスワードは、zoo2-infra の `github/zoo2-desktop-tauri/updater-signing-key.sops.yaml` に sops で暗号化して置き、これを正本にする（GitHub Secrets の値は後から読み出せないため）。GitHub Secrets へは、そこから `sops decrypt --extract` の出力を `gh secret set` に渡して登録する。手順は zoo2-infra の `github/README.md` にある。秘密鍵をなくすと、配った版はそれ以降の更新を受け付けない。
- **後続の項目**:
  - `requireSignedVersion`（署名に版を含め、古い版への差し戻しを防ぐ）は入れていない。いまの tauri-cli（2.11.4）は署名に版を書かないため。CLI を上げたら有効にする。

### macOS の署名と公証

新規リリースの macOS 版は、Developer ID Application 証明書で署名し、Apple の公証と公証チケットの添付を行う。Tauri CLI が証明書を一時キーチェーンへ取り込み、署名・公証を処理する。`bundle.macOS.hardenedRuntime` は有効にし、マイクとカメラの権限は既存の `Entitlements.plist` を使う。

GitHub の Environment `desktop-tauri-release` に、次の Secrets をすべて登録する。これらは Apple への署名・公証に使うもので、自動更新用の `TAURI_SIGNING_PRIVATE_KEY` とは別に必要になる。

| Secret | 値 |
| --- | --- |
| `APPLE_CERTIFICATE` | 証明書と秘密鍵を含む `.p12` を Base64 にした文字列 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` の書き出し時に設定した空でないパスワード |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: 氏名または組織名 (TEAMID)` という証明書の完全な名前 |
| `APPLE_ID` | 公証に使う Apple Account のメールアドレス |
| `APPLE_PASSWORD` | そのアカウントの App 用パスワード。通常のログインパスワードは使わない |
| `APPLE_TEAM_ID` | 証明書を発行した Apple Developer チームの Team ID |

新規リリースで 1 つでも未設定なら、ビルド前にエラーで止める。ad-hoc 署名には切り替えない。署名・公証後に `codesign --verify`、`xcrun stapler validate`、`spctl --assess` を実行し、失敗した配布物はアップロードしない。この確認が済んだ `.app` が DMG と自動更新用の `.app.tar.gz` に含まれる。既存リリースの `latest.json` だけを修復する場合は、Apple の Secrets を必要としない。

macOS の配布ビルドで `codesign` が `A timestamp was expected but was not found.` を返した場合だけ、30 秒・60 秒待って最大 2 回再試行する。タイムスタンプ取得の一時的な失敗で、ビルド環境の準備からやり直すことを避けるため。毎回同じ設定で Tauri のビルド・署名・公証を実行し、最後まで失敗した場合や別のエラーでは処理を止める。タイムスタンプや署名・公証の検証は省略しない。

初回の準備は次の順で行う。

1. Apple Developer Program に加入し、Account Holder が [Developer ID Application 証明書を発行する](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/)。Mac のキーチェーンアクセスで CSR を作り、Apple Developer に送る。発行された `.cer` を、CSR を作った Mac のキーチェーンに登録する。
2. キーチェーンアクセスの「自分の証明書」から、証明書と秘密鍵をパスワード付き `.p12` として書き出す。`Apple Development` や `Apple Distribution` は使わない。
3. `security find-identity -v -p codesigning` で証明書の名前を確認する。[Apple Account](https://account.apple.com/) で公証用の App 用パスワードを発行する。
4. 上表の Secrets を Environment に登録する。`.p12` とパスワードはリポジトリに置かず、値をログやチャットへ出さない。証明書の登録は次のように、Base64 の出力を直接渡せる。

   ```bash
   openssl base64 -A -in /path/to/developer-id.p12 | \
     gh secret set APPLE_CERTIFICATE --repo Zli-UoA/zoo2 --env desktop-tauri-release
   ```

5. Secrets の登録後に `desktop-tauri/src-tauri/Cargo.toml` と対応する `Cargo.lock` の版を上げ、通常のレビューを経て main に入れる。同じ版の既存リリースは作り直さないため、初回も新しい版が必要になる。最初の公証は時間がかかる場合がある。失敗したときは公証結果を確認してから再実行し、署名や公証の検証を省略しない。

複数の配布変更を並行して進める場合は、PR ごとの版と main へ入れる順を確認する。同じ版を使うと後から入れた変更はビルドされないため、直前に公開する版より大きい未使用の版を `Cargo.toml` と `Cargo.lock` にそろえて指定する。マージ順を変える場合は版の割り当ても見直す。

PR の作業ブランチで実ビルドの署名・公証を確認するときは、push 後に検証専用モードを手動実行する。

```bash
gh workflow run cd-desktop-tauri-release.yaml --repo Zli-UoA/zoo2 \
  --ref "$(git branch --show-current)" -f verify_only=true
```

検証専用モードでは、同じ版のリリースが存在していても macOS universal 版と Windows インストーラー版を作る。本番と同じ証明書・公証・自動更新用署名と `latest.json` 生成・検証処理（公開鍵の埋め込み確認、各プラットフォームの `.sig` 検証）を通り、生成された成果物と `zoo2-desktop-tauri-verify-manifest` を Actions の artifact に保存する。GitHub Release と配布用の `latest.json` は更新しない。main 以外での通常の公開実行は引き続き拒否する。署名用 Secrets にアクセスできるため、実行前に対象ブランチの変更を確認する。

ローカルで署名する場合は、証明書と秘密鍵がキーチェーンにあれば `.p12` の環境変数は不要。`APPLE_SIGNING_IDENTITY` と公証用の `APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` を環境変数に設定してから、Mac の開発環境でビルドする。

```bash
cd desktop-tauri/src-tauri
cargo tauri build --bundles app,dmg -- --locked
```

PR の検証用 CI（`ci-desktop-tauri.yaml`）は、署名用 Secrets を渡さず、引き続き ad-hoc 署名でビルドを確認する。v0.2.10 以前の公開済みリリースにも Developer ID 署名・公証はない。

設定の根拠は [Tauri の macOS 署名手順](https://v2.tauri.app/distribute/sign/macos/) と [環境変数の一覧](https://v2.tauri.app/reference/environment-variables/) を参照する。

## 未確定・確認が要るもの

- AuthZは`offline_access`に対応している。「Zoo for Desktop」で更新トークンを利用するには、登録済みclientに同scopeを許可し、デバイス認可で要求して本人が同意する。Web用の`VITE_OIDC_SCOPE`とは別に、デスクトップは共通クライアントの`ZOO_OIDC_SCOPE`（既定値は`openid profile offline_access`）を使う。発行・rotation・再起動後の復元は実機で確認する。`TokenManager`は更新トークンがない場合も動き、期限が切れたらログインし直しになる。
- ループバックの redirect（RFC 8252）：ログインはデバイスコードなので依存しないが、E2EE の再認証（`zoo_reauthenticate`）は依存する。「Zoo for Desktop」に認可コードのグラントとループバックのリダイレクト URI を AuthZ の管理画面で足し、zoo2-infra の `OIDC_REAUTH_AUDIENCES` にこのクライアント ID を足すまでは、再認証が要る保存は 403 になる。

## 開発

`nix develop` の中で動かす。Tauri の CLI、trunk、wasm-bindgen、WebKitGTK が入っている。

```sh
cd desktop-tauri/src-tauri
cargo tauri dev                    # trunk serve (localhost:1420) と一緒に起動する
cargo tauri build --bundles deb    # NixOS では AppImage を作れないので deb だけ。AppImage は CI で作る
```

- 画面の案は `desktop_ui`（`ZOO_DESKTOP_UI`）の `remote|leptos` で選ぶ。既定は `remote`（Web 版の画面。通話もこちらでだけ使える）。ただし `ZOO_DEV_USER` を付けたとき（dev 認証）は `leptos` にする。ローカルの backend は Web の画面を配信しないため。`desktop_ui` は `client-core` の `desktop_ui_for` で Tauri 版だけが読み、`Config` には入れない（値を誤っても TUI や GPUI 版の起動を止めないように）。知らない値は設定のエラーとして同梱の画面に出す（打ち間違いに気づけるように）。TUI と GPUI 版は読まないので影響しない。
- macOS の署名は ad-hoc でも Hardened Runtime になるので、マイクとカメラの entitlement（`src-tauri/Entitlements.plist`、`bundle.macOS.entitlements`）が要る。CI の macOS のジョブで、署名したアプリに入っていることを `codesign` で確かめる。
- 通話と、他のユーザーの検証の QR コードの読み取り（`docs/e2ee.md` の「QR コードによる検証」）: (b) の画面にだけ、マイクとカメラの許可を与える（`on_permission_request`）。macOS は `src-tauri/Info.plist` の `NSMicrophoneUsageDescription` と `NSCameraUsageDescription` で OS が使用を尋ねる（カメラの説明文は、通話と QR の読み取りの両方を書く）。Linux は WebKitGTK の WebRTC と media stream を既定で無効にしているので、ウィンドウを作ったあとに有効にする。
- 通知のオン・オフ: Web 版の設定画面の「通知」から `zoo_notification_settings` / `zoo_set_notifications` で読み書きし、Rust 側が `notifications.json` に保存する。オフのときは投稿の通知を出さない（更新の知らせは出す）。古い版のアプリにはこのコマンドがないので、Web 側は呼べなかったら説明だけを出す（`PROTOCOL_VERSION` は上げない）。デスクトップ版では Web Push を使わないので、設定画面のプッシュ通知の行は出さない。
- macOS の通知は、notify-rust の `preview-macos-un` で UNUserNotificationCenter から出し、起動時に許可を求める（古い NSUserNotification は許可を求められず、Finder として出そうとして表示されないことがある）。
- (b) で読み込む先は `api_base_url` の起点。debug ビルドだけ、`ZOO_DESKTOP_REMOTE_URL=http://localhost:5173` で Vite の dev サーバーに差し替えられる（React 側の変更をデプロイ前に試すため）。
- 接続先とログインは TUI と同じ環境変数（`ZOO_API_BASE_URL`、`ZOO_OIDC_CLIENT_ID`、`ZOO_OIDC_RESOURCE` など）か、`~/.config/zoo2-desktop-tauri/config.toml`（Windows では `%APPDATA%\zoo2-desktop-tauri\config\config.toml`）で変える。
- Windows のリリース構成をローカルで作るときは、Rust の `wasm32-unknown-unknown` ターゲット、`trunk`、`wasm-bindgen-cli`、`tauri-cli`（版は `ci-desktop-tauri.yaml` と合わせる）、Inno Setup 6.5.2 以降、WebView2 Runtime、Windows C++ ビルドツールを用意する。`desktop-tauri/src-tauri` で `cargo tauri build --no-bundle -- --locked` を実行したあと、リポジトリルートで `iscc /DAppVersion=<版> desktop-tauri\installer\zoo2.iss` を実行する（Windows の Authenticode コード署名は未導入のため、初回実行時に SmartScreen の確認が出る）。
- ローカルの backend（`DEV_AUTH_ENABLED=true`）に向けるときは `ZOO_DEV_USER=<UUID>` を付ける。デバイスコードのログインを通らずに `X-Dev-User` で接続する。画面は同梱の `leptos` になる。Web の画面を試すときは、frontend の dev サーバー（`pnpm dev`）を立てて、debug ビルドで `ZOO_DESKTOP_UI=remote ZOO_DESKTOP_REMOTE_URL=http://localhost:5173` を付ける。
- NVIDIA と Wayland の組み合わせで画面が出ないときは、`WEBKIT_DISABLE_DMABUF_RENDERER=1` を付ける。
- `wasm-bindgen` は `desktop-tauri/ui/Cargo.toml` で、nixpkgs の `wasm-bindgen-cli` と同じ版に固定している。flake.lock を更新して版が変わったら、両方をそろえる。
