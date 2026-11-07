# zoo2 Desktop

zoo2 のネイティブデスクトップクライアントです。Rust と [GPUI](https://www.gpui.rs) で実装しています（macOS / Windows / Linux 対応）。

- チャンネル・ダイレクトメッセージ一覧、タイムライン、スレッドを最大3ペインで表示します。投稿、返信、編集、削除、リアクション、チャンネルへの参加と退出ができます。
- 読み取りは Web 版・TUI 版と同じく、backend のシェイププロキシ (`/api/shapes/*`) を Electric の HTTP プロトコルで購読します。
- 書き込み、既読、入力中は REST API を使います。未読インジケーターと、入力中のユーザーも表示します。
- ログインは AuthZ のデバイスコード方式 (RFC 8628) です。GUI のログイン画面または `zoo2-desktop login` コマンドからブラウザを開いて承認できます。

## 目次

- [初期設定](#初期設定)
- [ログインと起動](#ログインと起動)
- [操作方法](#操作方法)
- [設定項目](#設定項目)
- [開発](#開発)

## 初期設定

設定ファイルは `~/.config/zoo2-desktop/config.toml`（macOS では `~/Library/Application Support/zoo2-desktop/config.toml`、Windows では `%APPDATA%\zoo2-desktop\config.toml`）から読み込みます。環境変数 (`ZOO_*`) でも指定でき、環境変数が優先されます。

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
| `zoo2-desktop help` | 使い方を表示する |

- トークンは設定ファイルと同じディレクトリの `token.json` に、所有者だけが読める権限 (Unix では `0600`) で保存します。
- アクセストークンは期限の 60 秒前に `refresh_token` で自動更新します。

## 操作方法

| 操作 | 内容 |
|---|---|
| サイドバー上部の検索欄 | チャンネル・DM を名前で絞り込む (`Enter` で先頭を開く、`Esc` でクリア) |
| サイドバーのチャンネル / DM クリック | チャンネルを開いてタイムラインを表示・既読化 |
| メッセージの「返信」 / 「💬 返信 N 件を表示」 | 右側にスレッドペインを開き、送信先をスレッド返信にする |
| メッセージの「+ リアクション」 | クイック絵文字 (`👍` `🎉` `❤️` `👀` `😂` `🙏`) またはショートコード入力 (`thumbsup`、`:tada:` など) でリアクションを追加・解除 |
| 自分のメッセージの「編集」 / 「削除」 | メッセージ本文の編集、または確認バーからの削除 |
| 入力欄で `Enter` | メッセージ送信（日本語 IME 変換中は確定のみ） |
| 入力欄で `Shift+Enter` / `Alt+Enter` | 改行を挿入 |
| 入力欄で `Esc` | 返信・編集ターゲットをキャンセルして通常投稿に戻る |

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
