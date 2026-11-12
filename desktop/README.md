# zoo2 Desktop

zoo2 のネイティブデスクトップクライアントです。Rust と [GPUI](https://www.gpui.rs) で実装しています（macOS / Windows / Linux 対応）。

- チャンネル・ダイレクトメッセージ一覧、タイムライン、スレッドを最大3ペインで表示します。投稿、返信、編集、削除、リアクション、チャンネルへの参加と退出ができます。
- 読み取りは Web 版・TUI 版と同じく、backend のシェイププロキシ (`/api/shapes/*`) を Electric の HTTP プロトコルで購読します。
- 書き込み、既読、入力中は REST API を使います。未読インジケーターと、入力中のユーザーも表示します。
- ログインは AuthZ のデバイスコード方式 (RFC 8628) です。GUI のログイン画面または `zoo2-desktop login` コマンドからブラウザを開いて承認できます。

この crate は `frontend/` のトークン・翻訳を埋め込むので、リポジトリの中でしかビルドできません (crates.io には公開しません)。

## 目次

- [初期設定](#初期設定)
- [ログインと起動](#ログインと起動)
- [操作方法](#操作方法)
- [設定項目](#設定項目)
- [開発](#開発)
- [デザインの土台とギャラリー](#デザインの土台とギャラリー)

## 初期設定

設定ファイルは `~/.config/zoo2-desktop/config.toml`（macOS では `~/Library/Application Support/zoo2-desktop/config.toml`、Windows では `%APPDATA%\zoo2-desktop\config\config.toml`）から読み込みます。環境変数 (`ZOO_*`) でも指定でき、環境変数が優先されます。

### OIDC クライアント ID (`ZOO_OIDC_CLIENT_ID`) の指定について

本番の AuthZ (`https://account.zli.works/api/auth`) でデスクトップ版専用の public client ID が発行・既定値に反映されるまでは、AuthZ に登録されている public client の ID を `config.toml` の `oidc_client_id` または環境変数 `ZOO_OIDC_CLIENT_ID` で指定してください。

使用する public client には、AuthZ 側で以下の設定が必要です。

- デバイスコード (`urn:ietf:params:oauth:grant-type:device_code`) と `refresh_token` グラントが有効であること
- リソース `https://zli.chat/api`（backend の `OIDC_AUDIENCE` と同じ値）が紐付けられていること
- スコープ `openid profile offline_access` が許可されていること

```toml
api_base_url = "https://zli.chat"
oidc_client_id = "<AuthZ で発行された public client ID>"
```

または環境変数で指定して起動します。

```bash
ZOO_OIDC_CLIENT_ID=<AuthZ で発行された public client ID> zoo2-desktop
```

## ログインと起動

```bash
zoo2-desktop
```

未ログインのときはログイン画面が開きます。「ブラウザでログイン」を押すと確認コードが表示され、ブラウザが開きます。AuthZ でコードを確かめて承認すると、そのままチャット画面へ切り替わります。

CLI からログイン・ログアウトすることもできます。

| コマンド | 内容 |
|---|---|
| `zoo2-desktop` / `zoo2-desktop run` | デスクトップアプリのウィンドウを開く |
| `zoo2-desktop login` | デバイスコードでログインする |
| `zoo2-desktop logout` | 手元のトークンを消し、IdP 側の `refresh_token` を失効させる |
| `zoo2-desktop gallery` | 色・フォント・アイコンの確認画面を開く |
| `zoo2-desktop help` | 使い方を表示する |

- トークンは設定ファイルと同じディレクトリの `token.json` に、所有者だけが読める権限 (Unix では `0600`) で保存します。
- アクセストークンは期限の 60 秒前に `refresh_token` で自動更新します。

## 操作方法

| 操作 | 内容 |
|---|---|
| `⌘K` | クイックチャンネルスイッチャーを開く（チャンネル・DM を前方一致・部分一致で素早く切り替え） |
| `⌘N` / サイドバーの `+` ボタン | チャンネル作成モーダルを開く（名前・説明・非公開フラグを指定して作成） |
| `⌘F` / タイムラインヘッダーの `🔍` ボタン | チャンネル内メッセージ検索バーを開閉（`↑` / `↓` で一致メッセージ間を移動・強調スクロール、`Esc` で閉じる） |
| サイドバー上部の検索欄 | チャンネル・DM を名前で絞り込む (`Enter` で先頭を開く、`Esc` でクリア) |
| サイドバーのチャンネル / DM クリック | チャンネルを開いてタイムラインを表示・既読化（メッセージ編集中は保存または `Esc` でキャンセルするまで切り替え不可） |
| `⌘T` / サイドバー下部のテーマ切り替え | ダークテーマとライトテーマを切り替える（クリックまたはフォーカスして `Enter` / `Space`、ショートカット `⌘T` / `⌘⇧T`） |
| メッセージの「返信」 / 「💬 返信 N 件を表示」 | 右側にスレッドペインを開き、送信先をスレッド返信にする |
| メッセージの「+ リアクション」 | クイック絵文字 (`👍` `🎉` `❤️` `👀` `😂` `🙏`) またはショートコード入力 (`thumbsup`、`:tada:` など) でリアクションを追加・解除 |
| 自分のメッセージの「編集」 / 「削除」 | メッセージ本文の編集（編集中は他の編集や別チャンネルへの移動を保護）、または確認バーからの削除 |
| 入力欄で `Enter` | メッセージ送信（日本語 IME 変換中は確定のみ） |
| 入力欄で `Shift+Enter` / `Alt+Enter` | 改行を挿入 |
| 入力欄で `Esc` | 返信・編集ターゲットをキャンセルして通常投稿に戻る、またはモーダル・検索を閉じる |
| `⌘U` / コンポーザーの `📎` ボタン / ドラッグ＆ドロップ | ファイル選択ダイアログまたはウィンドウへのドラッグ＆ドロップでファイルを添付（最大10件、1ファイル50MBまで。編集中は無効。空ファイルおよび危険な実行可能拡張子は除外） |
| `⌘E` / コンポーザーの `😀` ボタン | 絵文字パレットを開閉し、矢印キーや `Enter` / `Space` で選択絵文字をキャレット位置へ挿入 |
| 入力欄で `@` 入力 | メンバーや `@channel`, `@here` のメンション補完ポップオーバーを表示（`↑` / `↓` で選択、`Enter` / `Tab` で確定） |
| タイムライン上の画像 / 添付ファイル | 画像はインライン表示され、クリックでブラウザ表示。ファイルはピルとして表示され、クリックでダウンロード |

### OS ネイティブ通知の対象と制約について

- ウィンドウがバックグラウンド（非アクティブ）の間に、**現在開いているチャンネル**に届いた自分宛てメンション（`@username`, `@channel`, `@here`）およびダイレクトメッセージ（DM）の新着メッセージを OS ネイティブ通知で通知します。
- **制約事項:** Electric shape の購読対象が開いているチャンネルに限られるため、開いていない別チャンネルや別 DM の新着通知は本バージョンでは対象外です（今後の通知専用 shape 導入等で対応予定）。また、初回同期時やメッセージ編集時の大量通知は抑制されます。

## 設定項目

`config.toml` と環境変数から読み込みます。環境変数が優先されます。

| `config.toml` | 環境変数 | 既定値 | 内容 |
|---|---|---|---|
| `api_base_url` | `ZOO_API_BASE_URL` | `https://zli.chat` | backend の URL |
| `dev_user` | `ZOO_DEV_USER` | なし | dev モードのユーザー UUID。指定したときだけ `X-Dev-User` で接続し、OIDC ログインは使わない |
| `oidc_issuer` | `ZOO_OIDC_ISSUER` | `https://account.zli.works/api/auth` | AuthZ の issuer |
| `oidc_client_id` | `ZOO_OIDC_CLIENT_ID` | `zoo2-desktop` | AuthZ に登録した public client の ID |
| `oidc_resource` | `ZOO_OIDC_RESOURCE` | `https://zli.chat/api` | 要求する resource (RFC 8707)。backend の `OIDC_AUDIENCE` と同じ値にする |
| `oidc_scope` | `ZOO_OIDC_SCOPE` | `openid profile offline_access` | 要求するスコープ |
| `check_updates` | `ZOO_CHECK_UPDATES` | `true` | 起動時に新しい版 (`desktop-v*`) を調べる。`false` で調べない |

## 開発

Nix devShell（リポジトリルートで `nix develop`）を使います。

### ローカルの backend で試す (dev モード)

```bash
make db-up && make migrate
make backend-dev
make desktop-dev
```

`make desktop-dev` は `http://localhost:8000` の backend に、既定で `00000000-0000-0000-0000-000000000001` のユーザーとして接続します。別のユーザーで試すときは `ZOO_DEV_USER` を指定します。

```bash
ZOO_DEV_USER=<ユーザーの UUID> make desktop-dev
```

### チェック

```bash
make desktop-check
```

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` を実行します。

## デザインの土台とギャラリー

見た目は Web 版 (`frontend/`) にそろえます。そのための部品を `src/` に置いています。

| ファイル | 内容 |
|---|---|
| `tokens.rs` | `frontend/src/styles.css` の色変数 (Light は `:root`、Dark は `.dark`)、角丸、文字サイズ。`oklch` は sRGB に変換して持つ |
| `fonts.rs` | 同梱の Geist と Noto Sans JP (どちらも Regular / Medium / SemiBold / Bold の静的 TTF) を登録する。かなと漢字は Noto Sans JP で描く |
| `icons.rs` | lucide のアイコン。SVG は `assets/icons/` にある |
| `i18n.rs` | `frontend/src/lib/i18n/locales/*.json` を埋め込んだ翻訳。言語は OS のロケールから決め、見つからなければ ja |
| `assets.rs` | `assets/` (フォント、アイコン、ロゴ) を実行ファイルに埋め込む |

- 色の値は `tokens.rs` のテストが `styles.css` を読んで突き合わせます。CSS に色変数を足したのに `tokens.rs` に無いときは、テストが失敗します。
- アイコンを足すときは、`node desktop/scripts/collect-lucide-icons.mjs` で frontend の import から集め直し、`icons.rs` の `Icon` に足します。版は frontend の `lucide-react` に合わせています。
- フォントとアイコンのライセンスは `assets/NOTICE` にあります。

### ギャラリー

```bash
make desktop-gallery              # 明るい配色
THEME=dark make desktop-gallery   # 暗い配色
```

`zoo2-desktop gallery --theme light|dark` と同じです。`--theme` は環境変数 `ZOO_DESKTOP_THEME` でも指定できます。

### 画面の撮影 (Linux)

仮想ディスプレイ (Xvfb) と openbox の上で動かし、root ウィンドウを PNG にします。`nix develop` の中で実行してください。

```bash
cd desktop && cargo build
cd .. && scripts/desktop-screenshot.sh /tmp/gallery-dark.png -- desktop/target/debug/zoo2-desktop gallery --theme dark
```

`-w 秒` で待ち時間、`-s 幅x高さ` で画面の大きさを変えられます (既定は 6 秒、1440x900)。openbox が無いと GPUI のウィンドウは黒く描かれます。描画は lavapipe (CPU の Vulkan) で、devShell の `ZOO_LVP_ICD` を使います。

スクリプトは自分で起動したプロセスの PID だけを止めます。`cargo run` で起動すると、アプリは cargo の子プロセスになって残ることがあるので、ビルドしたあとに `desktop/target/debug/zoo2-desktop` を直接渡してください。
