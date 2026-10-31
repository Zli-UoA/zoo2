# zoo2 TUI

zoo2 をターミナルから使うチャットクライアントです。Rust と [ratatui](https://ratatui.rs) で書いています。

- 読み取りは Web 版と同じく、backend のシェイププロキシ (`/api/shapes/*`) を Electric の HTTP プロトコルで購読します。ほかの人の投稿や編集はすぐに反映されます。
- 書き込み、既読、入力中は REST API を使います。
- ログインは AuthZ のデバイスコード方式 (RFC 8628) です。端末に出た URL とコードをブラウザで承認します。

## 必要環境

- Nix devShell (リポジトリルートで `nix develop`)。Rust のバージョンはルートの `rust-toolchain.toml` に従います。
- 接続先の backend。ローカルで試すときは backend を dev モードで起動します。

## ローカルで試す (dev モード)

backend の `DEV_AUTH_ENABLED=true` を使い、`X-Dev-User` でユーザーを指定して接続します。ログインは不要です。

```bash
make db-up && make migrate
```

```bash
make backend-dev
```

```bash
make tui-dev
```

`make tui-dev` は既定で `00000000-0000-0000-0000-000000000001` のユーザーとして接続します。別のユーザーで試すときは `ZOO_DEV_USER` を指定します。

```bash
ZOO_DEV_USER=<ユーザーの UUID> make tui-dev
```

## 設定

`~/.config/zoo2-tui/config.toml` と環境変数から読み込みます。環境変数が優先されます。

| `config.toml` | 環境変数 | 既定値 | 内容 |
|---|---|---|---|
| `api_base_url` | `ZOO_API_BASE_URL` | `http://localhost:8000` | backend の URL |
| `dev_user` | `ZOO_DEV_USER` | なし | dev モードのユーザー。指定すると OIDC より優先される |
| `oidc_issuer` | `ZOO_OIDC_ISSUER` | なし | AuthZ の issuer |
| `oidc_client_id` | `ZOO_OIDC_CLIENT_ID` | なし | AuthZ に登録した TUI 用クライアントの ID |
| `oidc_resource` | `ZOO_OIDC_RESOURCE` | なし | 要求する resource (RFC 8707)。backend の `OIDC_AUDIENCE` と同じ値にする |
| `oidc_scope` | `ZOO_OIDC_SCOPE` | `openid profile offline_access` | 要求するスコープ |

`oidc_issuer` と `oidc_client_id` の両方があれば OIDC、どちらもなければ dev モードで接続します。

本番に接続する例:

```toml
api_base_url = "https://zli.chat"
oidc_issuer = "https://account.zli.works/api/auth"
oidc_client_id = "<TUI 用クライアントの ID>"
oidc_resource = "https://zli.chat"
```

## ログイン

```bash
cargo run -- login
```

端末に URL とコードが出て、ブラウザが開きます。AuthZ にログインし、コードが一致することを確かめて承認すると、端末側のログインが終わります。

- トークンは `~/.config/zoo2-tui/token.json` に、所有者だけが読める権限 (0600) で保存します。
- アクセストークンは期限の 60 秒前に refresh_token で更新します。更新できないときは、再ログインを求めるメッセージが出ます。
- `cargo run -- logout` で、IdP 側の refresh_token を失効させてから手元のトークンを消します。

AuthZ には、デバイスコードと refresh_token のグラントを許可した public client を登録しておく必要があります。

## キー操作

| キー | 操作 |
|---|---|
| `Tab` / `Shift+Tab` | ペインを移動 |
| `j` `k` / `↓` `↑` | 選択を移動 |
| `g` / `G` | 先頭 / 末尾へ |
| `Enter` | チャンネルを開く / スレッドを開く |
| `Esc` | 選択を外す / スレッドを閉じる |
| `i` | 書く (スレッドでは返信) |
| `Enter` (入力中) | 送信 |
| `Alt+Enter` (入力中) | 改行 |
| `e` | 自分のメッセージを編集 |
| `d` | 自分のメッセージを削除 |
| `r` | リアクションを付ける / 外す (`thumbsup`、`:tada:`、`👍` など) |
| `J` / `L` | チャンネルに参加 / 退出 |
| `/` | チャンネルを検索 |
| `?` | キー操作の一覧 |
| `q` / `Ctrl+C` | 終了 |

## 表示について

- 本文の MFM は、ターミナルで表せる装飾 (太字、斜体、取り消し線、コード、引用、リンク、メンション、ハッシュタグ、絵文字ショートコード) だけを反映します。`$[tada ...]` などの関数は中身の文字だけを表示します。
- 時刻は端末のタイムゾーンで表示します。
- 添付ファイル、通知画面、チャンネルの作成と管理には、まだ対応していません。

## 開発

```bash
make tui-check
```

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` を実行します。テストは backend を使わず、HTTP の応答を wiremock で、画面を ratatui の `TestBackend` で確かめます。

## 構成

- `src/config.rs`: 設定の読み込み
- `src/http.rs`: 認証ヘッダーとエラー応答
- `src/api.rs`: REST API
- `src/electric/`: Electric のシェイプ購読と、行の型変換
- `src/store.rs`: 行の保持と、画面用の派生ビュー
- `src/mfm.rs`: MFM の簡易レンダラ
- `src/auth/`: デバイスコードログインとトークンの保存・更新
- `src/ui/`: 画面の状態、キー操作、描画
