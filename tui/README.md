# zoo2 TUI

zoo2 をターミナルから使うチャットクライアントです。Rust と [ratatui](https://ratatui.rs) で書いています。

- チャンネル一覧、タイムライン、スレッドを3ペインで表示します。投稿、返信、編集、削除、リアクション、チャンネルへの参加と退出ができます。
- 読み取りは Web 版と同じく、backend のシェイププロキシ (`/api/shapes/*`) を Electric の HTTP プロトコルで購読します。ほかの人の投稿や編集はすぐに反映されます。
- 書き込み、既読、入力中は REST API を使います。未読のチャンネルと、入力中のユーザーも表示します。
- ログインは AuthZ のデバイスコード方式 (RFC 8628) です。端末に出た URL とコードをブラウザで承認するので、SSH 先のサーバーからでもログインできます。

## 目次

- [インストール](#インストール)
- [初期設定](#初期設定)
- [ログインと起動](#ログインと起動)
- [キー操作](#キー操作)
- [設定項目](#設定項目)
- [更新とアンインストール](#更新とアンインストール)
- [困ったとき](#困ったとき)
- [開発](#開発)

## インストール

ビルド済みのバイナリを [GitHub Releases](https://github.com/Zli-UoA/zoo2/releases) で配布しています。`tui/Cargo.toml` の `version` を上げて main に入れると、`tui-v<version>` のタグでリリースが作られます。

| ファイル | 対象 |
|---|---|
| `zoo2-tui_amd64.deb` / `zoo2-tui_arm64.deb` | Ubuntu、Debian |
| `zoo2-tui.x86_64.rpm` / `zoo2-tui.aarch64.rpm` | CentOS Stream、RHEL、Rocky Linux、AlmaLinux、Fedora |
| `PKGBUILD` | Arch Linux |
| `zoo2-tui-x86_64-linux.tar.gz` / `zoo2-tui-aarch64-linux.tar.gz` | そのほかの Linux |
| `zoo2-tui-x86_64-macos.tar.gz` / `zoo2-tui-aarch64-macos.tar.gz` | macOS (Intel / Apple シリコン) |
| `SHA256SUMS` | 上のファイルのチェックサム |

Linux のバイナリは musl で静的リンクしているので、glibc の版に関係なく動きます。TLS の証明書は rustls と同梱のルート証明書で検証するため、OpenSSL も要りません。

以下の URL の `releases/latest/download/` は最新のリリースを指します。特定の版を入れるときは `releases/download/tui-v0.1.0/` のように書き換えてください。

### Ubuntu / Debian

```bash
curl -LO https://github.com/Zli-UoA/zoo2/releases/latest/download/zoo2-tui_amd64.deb
```

```bash
sudo apt install ./zoo2-tui_amd64.deb
```

ARM64 のマシンでは `zoo2-tui_arm64.deb` を使います。`/usr/bin/zoo2-tui` に入ります。

### CentOS Stream / RHEL / Rocky Linux / AlmaLinux / Fedora

```bash
sudo dnf install https://github.com/Zli-UoA/zoo2/releases/latest/download/zoo2-tui.x86_64.rpm
```

ARM64 のマシンでは `zoo2-tui.aarch64.rpm` を使います。CentOS 7 のように dnf がない環境では、`yum install` に読み替えてください。

### Arch Linux

リリースに添付した `PKGBUILD` で、ビルド済みバイナリのパッケージ `zoo2-tui-bin` を作って入れます。AUR には登録していません。

```bash
mkdir zoo2-tui-bin && cd zoo2-tui-bin
```

```bash
curl -LO https://github.com/Zli-UoA/zoo2/releases/latest/download/PKGBUILD
```

```bash
makepkg -si
```

x86_64 と aarch64 に対応しています。`pacman -R zoo2-tui-bin` で消せます。

### NixOS / Nix

リポジトリの flake に `zoo2-tui` パッケージがあります。ソースからビルドするので、初回は時間がかかります。

試しに動かすだけなら、インストールせずに起動できます。

```bash
nix run github:Zli-UoA/zoo2#zoo2-tui -- help
```

ユーザーのプロファイルに入れるときは次のとおりです。

```bash
nix profile install github:Zli-UoA/zoo2#zoo2-tui
```

NixOS の設定 (flake) に入れるときは、`inputs` に zoo2 を足し、パッケージを `environment.systemPackages` か Home Manager の `home.packages` に加えます。

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    zoo2.url = "github:Zli-UoA/zoo2";
  };

  outputs = { nixpkgs, zoo2, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        ({ pkgs, ... }: {
          environment.systemPackages = [ zoo2.packages.${pkgs.system}.zoo2-tui ];
        })
      ];
    };
  };
}
```

特定のリリースに固定するときは `github:Zli-UoA/zoo2/tui-v0.1.0` のようにタグを指定します。

### macOS / そのほかの Linux

アーカイブを展開し、`PATH` の通ったディレクトリに置きます。

```bash
curl -LO https://github.com/Zli-UoA/zoo2/releases/latest/download/zoo2-tui-aarch64-macos.tar.gz
```

```bash
tar -xzf zoo2-tui-aarch64-macos.tar.gz zoo2-tui
```

```bash
sudo install -m 755 zoo2-tui /usr/local/bin/zoo2-tui
```

Intel の Mac では `zoo2-tui-x86_64-macos.tar.gz`、Linux では `zoo2-tui-x86_64-linux.tar.gz` か `zoo2-tui-aarch64-linux.tar.gz` に置き換えます。ブラウザでダウンロードしたファイルを macOS が開けないときは、`xattr -d com.apple.quarantine zoo2-tui` で隔離の属性を外してください。

### ソースからビルドする

新しい stable の Rust が要ります (rustup なら `rustup update stable`)。CI と配布物は、ルートの `rust-toolchain.toml` の版でビルドしています。

```bash
cargo install --locked --git https://github.com/Zli-UoA/zoo2 zoo2-tui
```

`~/.cargo/bin/zoo2-tui` に入ります。

### ダウンロードしたファイルを確かめる

```bash
curl -LO https://github.com/Zli-UoA/zoo2/releases/latest/download/SHA256SUMS
```

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

macOS では `shasum -a 256 --check --ignore-missing SHA256SUMS` を使います。

## 初期設定

`~/.config/zoo2-tui/config.toml` に接続先を書きます。macOS では `~/Library/Application Support/zoo2-tui/config.toml` です。

```toml
api_base_url = "https://zli.chat"
oidc_issuer = "https://account.zli.works/api/auth"
oidc_client_id = "<TUI 用クライアントの ID>"
oidc_resource = "https://zli.chat"
```

`oidc_client_id` には、AuthZ に登録した TUI 用の public client の ID を入れます。分からないときは zoo2 の管理者に確かめてください。そのほかの項目は [設定項目](#設定項目) を参照してください。

## ログインと起動

```bash
zoo2-tui login
```

端末に URL とコードが出て、ブラウザが開きます。AuthZ にログインし、コードが一致することを確かめて承認すると、端末側のログインが終わります。ブラウザを開けない環境 (SSH 先など) では、表示された URL を手元のブラウザで開いてください。

```bash
zoo2-tui
```

チャット画面が開きます。`zoo2-tui run` と同じです。`?` でキー操作の一覧、`q` で終了します。

| コマンド | 内容 |
|---|---|
| `zoo2-tui` / `zoo2-tui run` | チャット画面を開く |
| `zoo2-tui login` | デバイスコードでログインする |
| `zoo2-tui logout` | IdP 側の refresh_token を失効させてから、手元のトークンを消す |
| `zoo2-tui help` | 使い方を表示する |

- トークンは設定ファイルと同じディレクトリの `token.json` に、所有者だけが読める権限 (0600) で保存します。
- アクセストークンは期限の 60 秒前に refresh_token で更新します。更新できないときは、再ログインを求めるメッセージが出ます。

## キー操作

| キー | 操作 |
|---|---|
| `Tab` / `Shift+Tab` | ペインを移動 |
| `j` `k` / `↓` `↑` | 選択を移動 |
| `g` / `G` | 先頭 / 末尾へ |
| `Ctrl+D` / `Ctrl+U` (`PageDown` / `PageUp`) | ペインより長いメッセージの中を下 / 上へ |
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
| `R` | 送れなかった本文を入力欄に戻す (開いているチャンネルのものを優先) |
| `?` | キー操作の一覧 |
| `q` / `Ctrl+C` | 終了 |

## 表示について

- 本文の MFM は、ターミナルで表せる装飾 (太字、斜体、取り消し線、コード、引用、リンク、メンション、ハッシュタグ、絵文字ショートコード) だけを反映します。`$[tada ...]` などの関数は中身の文字だけを表示します。
- 時刻は端末のタイムゾーンで表示します。
- 添付ファイル、通知画面、チャンネルの作成と管理には、まだ対応していません。

## 設定項目

`config.toml` と環境変数から読み込みます。環境変数が優先されます。

| `config.toml` | 環境変数 | 既定値 | 内容 |
|---|---|---|---|
| `api_base_url` | `ZOO_API_BASE_URL` | `http://localhost:8000` | backend の URL |
| `dev_user` | `ZOO_DEV_USER` | なし | dev モードのユーザー。指定すると OIDC より優先される |
| `oidc_issuer` | `ZOO_OIDC_ISSUER` | なし | AuthZ の issuer |
| `oidc_client_id` | `ZOO_OIDC_CLIENT_ID` | なし | AuthZ に登録した TUI 用クライアントの ID |
| `oidc_resource` | `ZOO_OIDC_RESOURCE` | なし | 要求する resource (RFC 8707)。backend の `OIDC_AUDIENCE` と同じ値にする |
| `oidc_scope` | `ZOO_OIDC_SCOPE` | `openid profile offline_access` | 要求するスコープ |

`oidc_issuer` と `oidc_client_id` の両方があれば OIDC、どちらもなければ dev モードで接続します。

AuthZ には、デバイスコードと refresh_token のグラントを許可した public client を登録しておく必要があります。スコープは `openid profile offline_access`、resource は backend の `OIDC_AUDIENCE` と同じ値です。

## 更新とアンインストール

| 入れ方 | 更新 | アンインストール |
|---|---|---|
| deb | 新しい `.deb` を落として `sudo apt install ./zoo2-tui_amd64.deb` | `sudo apt remove zoo2-tui` |
| rpm | `sudo dnf install <新しい .rpm の URL>` | `sudo dnf remove zoo2-tui` |
| Arch Linux | 新しい `PKGBUILD` で `makepkg -si` | `sudo pacman -R zoo2-tui-bin` |
| nix profile | `nix profile upgrade zoo2-tui` | `nix profile remove zoo2-tui` |
| NixOS | `nix flake update zoo2` してから再ビルド | 設定から外して再ビルド |
| アーカイブ | 新しいバイナリで置き換える | `sudo rm /usr/local/bin/zoo2-tui` |
| cargo install | 同じコマンドをもう一度実行 | `cargo uninstall zoo2-tui` |

設定とトークンはパッケージを消しても残ります。完全に消すときは `~/.config/zoo2-tui` (macOS では `~/Library/Application Support/zoo2-tui`) を削除してください。消す前に `zoo2-tui logout` を実行すると、IdP 側のトークンも失効します。

## 困ったとき

- **「ログインしていません」と出る**: `zoo2-tui login` を実行してください。refresh_token が失効したときも同じです。
- **「OIDC が設定されていません」と出る**: `config.toml` に `oidc_issuer` と `oidc_client_id` があるか確かめてください。環境変数 `ZOO_DEV_USER` が残っていると dev モードになります。
- **ログインで `invalid_client` や `unauthorized_client` が出る**: `oidc_client_id` が違うか、そのクライアントにデバイスコードのグラントが許可されていません。管理者に確かめてください。
- **401 で画面が開かない**: `oidc_resource` が backend の `OIDC_AUDIENCE` と一致しているか確かめてください。
- **絵文字や罫線の幅がずれる**: 端末のフォントと、East Asian Ambiguous 幅の設定 (半角にする) を確かめてください。
- **画面の下に「同期を停止しました」と出る**: シェイプの購読がエラーで止まっています。メッセージやリアクションの購読はチャンネルを開き直すと作り直します。チャンネル一覧などが止まったときは、再起動してください。

## 開発

Nix devShell (リポジトリルートで `nix develop`) を使います。Rust のバージョンはルートの `rust-toolchain.toml` に従います。

### ローカルの backend で試す (dev モード)

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

### チェック

```bash
make tui-check
```

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` を実行します。テストは backend を使わず、HTTP の応答を wiremock で、画面を ratatui の `TestBackend` で確かめます。

### 構成

- `src/config.rs`: 設定の読み込み
- `src/http.rs`: 認証ヘッダーとエラー応答
- `src/api.rs`: REST API
- `src/electric/`: Electric のシェイプ購読と、行の型変換
- `src/store.rs`: 行の保持と、画面用の派生ビュー
- `src/mfm.rs`: MFM の簡易レンダラ
- `src/auth/`: デバイスコードログインとトークンの保存・更新
- `src/ui/`: 画面の状態、キー操作、描画

### リリース

1. `tui/Cargo.toml` の `version` を上げ、`Cargo.lock` を更新して (`cargo check`) PR を出します。
2. main に入ると [CD TUI Release](../.github/workflows/cd-tui-release.yaml) が動き、Linux (musl) と macOS 向けにビルドします。そして deb、rpm、Arch Linux 用の `PKGBUILD`、`SHA256SUMS` を添えて `tui-v<version>` のリリースを作ります。
3. 同じ version のリリースがすでにあるときは何もしません。version を上げずに `tui/` だけ変えても、リリースは作られません。
