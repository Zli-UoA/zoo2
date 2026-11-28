# zoo2 TUI

zoo2 をターミナルから使うチャットクライアントです。Rust と [ratatui](https://ratatui.rs) で書いています。

- チャンネル一覧、タイムライン、スレッドを3ペインで表示します。投稿、返信、編集、削除、リアクション、チャンネルへの参加と退出ができます。
- 参加チャンネルの下にアクティブなスレッドを表示します。ピン留めしたチャンネル（DM を除く）の行の下にも同じスレッドを表示し、その行で Enter を押してスレッドを開いても、カーソルは押した行（ピン留めの下か通常の一覧か）に残ります。アーカイブ済みスレッドは投稿の返信を開く操作から閲覧し、閲覧中もツリーには表示しません。一覧にないアクティブなスレッドを開いたときは、閲覧中の1件だけツリーに一時表示し、閉じると除外します。`u` で閲覧中のスレッドを再開できます。1日返信のないスレッドは自動でアーカイブされ、返信の投稿でも再開します。
- 読み取りは Web 版と同じく、backend のシェイププロキシ (`/api/shapes/*`) を Electric の HTTP プロトコルで購読します。ほかの人の投稿や編集はすぐに反映されます。
- 書き込み、既読、入力中は REST API を使います。未読のチャンネルと、入力中のユーザーも表示します。
- ログインは AuthZ のデバイスコード方式 (RFC 8628) です。端末に出た URL とコードをブラウザで承認するので、SSH 先のサーバーからでもログインできます。

## 目次

- [インストール](#インストール)
- [初期設定](#初期設定)
- [ログインと起動](#ログインと起動)
- [キー操作](#キー操作)
- [暗号化（初回設定と解錠）](#暗号化初回設定と解錠)
- [暗号化チャンネルと DM](#暗号化チャンネルと-dm)
- [端末と検証（失効、他の人の検証、解錠の手段）](#端末と検証失効他の人の検証解錠の手段)
- [設定項目](#設定項目)
- [更新とアンインストール](#更新とアンインストール)
- [困ったとき](#困ったとき)
- [開発](#開発)
- [ライセンス](#ライセンス)

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

設定は要りません。何も書かなくても、本番の zoo2 (`https://zli.chat`) に接続します。

接続先を変えたいときだけ、`~/.config/zoo2-tui/config.toml` に書きます。macOS では `~/Library/Application Support/zoo2-tui/config.toml` です。

```toml
api_base_url = "https://zli.chat"
```

そのほかの項目は [設定項目](#設定項目) を参照してください。

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
| `zoo2-tui logout` | この端末の暗号の鍵（`crypto/`）を消して E2EE の端末を削除し、IdP 側の refresh_token を失効させてから、手元のトークンを消す。終わっていない失効や、鍵バックアップに送れていない鍵があれば、示して確かめる |
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
| `Enter` | チャンネルを開く / 選択中の投稿・返信を親とするスレッドを開く |
| `Esc` | 選択を外す / スレッドを閉じる |
| `o` | 選択中のメッセージのチャンネル・メッセージのリンクを開く (いくつもあれば選ぶ) |
| `u` | 選択中のスレッドを再開 |
| `i` | 書く (スレッドでは返信) |
| `Enter` (入力中) | 送信 |
| `Alt+Enter` (入力中) | 改行 |
| `e` | 自分のメッセージを編集 |
| `d` | 自分のメッセージを削除 |
| `r` | リアクションを付ける / 外す (`thumbsup`、`:tada:`、`👍` など) |
| `p` | ピン留め / 解除 |
| `[` / `]` (`Alt+↑` / `Alt+↓`) | ピン留めを上 / 下へ (ピン留め選択中) |
| `J` / `L` | チャンネルに参加 / 退出 |
| `N` | 開いているチャンネルの通知レベルを変更 (すべて → メンション → ミュート) |
| `/` | チャンネル・アクティブなスレッドを検索 |
| `S` / `Ctrl+F` | メッセージ・ファイルを全文検索（既定は両方。暗号化チャンネルは対象外。Tab で種類（すべて・メッセージ・ファイル）、末尾で ↓ を押すと古い月も検索。語に `from:@名前`・`in:#チャンネル`（複数書くと、どれか）・`after:YYYY-MM-DD`・`before:YYYY-MM-DD`・`with:me`（自分が参加中の会話）・`-is:bot`（ボットを除く）を書くと絞り込む） |
| `R` | 送れなかった本文を入力欄に戻す (開いているチャンネルのものを優先) |
| `c` / `m` | チャンネル / DM を作る (`Tab` で公開範囲・暗号化を切り替える) |
| `V` | 暗号化チャンネルの配布先の詳細 (増えた配布先と、鍵を配れない相手の全員。`j` `k` でスクロール) |
| `E` | 暗号化の画面 (初回設定、リカバリーキーでの解錠) |
| `D` | 端末と検証の画面 (自分の端末の一覧と失効、他の人の検証、解錠の手段) |
| `?` | キー操作の一覧 |
| `q` / `Ctrl+C` | 終了 |

## 暗号化（初回設定と解錠）

暗号化チャンネル（E2EE）の鍵は `E` の画面で扱います。まだ設定していない、または解錠が必要なときは、ステータス行に「暗号化: 未設定 (E)」「暗号化: 解錠が必要 (E)」と出ます。

- 初回設定: 鍵を作り、リカバリーキーを表示します。紙やパスワードマネージャーに控えてから `y` を押すと、保存の直前にブラウザーが開くので、ログインし直してください（再認証）。再認証の戻り先は、この端末の `http://127.0.0.1:<空いているポート>/callback` です。
- 解錠: 別の端末で設定済みなら、控えたリカバリーキーを入力します（貼り付けもできます）。TUI ではパスキーは使えません。
- 初回設定が途中で止まったとき（保存の途中で終了したなど）は、もう一度 `E` を開き、控えたリカバリーキーで解錠すると、同じ鍵のまま続きを保存します。

SSH の先で TUI を動かしていて、ブラウザーが手元のマシンにあると、ブラウザーから戻り先に届きません。画面に出る `ssh -L <ポート>:127.0.0.1:<ポート> <ホスト>` を手元で実行してポートを転送するか、初回設定を Web か Android で行ってください。解錠はリカバリーキーだけでできるので、SSH の先でも行えます。

## 暗号化チャンネルと DM

- 作る: `c`（チャンネル）か `m`（DM。相手の `@ユーザー名` を空白で区切って入れる）を押し、`Tab` で「非公開・エンドツーエンドで暗号化する」を選んで `Enter` を押します。暗号化はあとから解除できません。既定では暗号化しません。
- 送る: 暗号化チャンネルでは、本文を送る直前にこの端末で暗号化し、メンバーの検証済みの端末あてに鍵を配ります。サーバーには平文を送りません。送るには、`E` の画面で初回設定か解錠を済ませておく必要があります。
- 前回の送信から増えた配布先と、鍵を配れない相手と理由（暗号化を設定していない、再検証が必要など）があれば、入力欄の上に出します。人数が多くて収まらないときは人数だけを出すので、`V` で全員を確かめてください。配布先の数（何人の何台の端末か）は `V` の画面で確かめられます。メンバーの一覧はサーバーが決めるものなので、知らない人が増えていないか確かめてください。
- 読む: 届いたメッセージはこの端末で復号して表示します。鍵がまだ届いていないメッセージは「鍵がまだ届いていません」と出し、鍵が届くと読めるようになります。送信端末を検証できないものには「未検証の送信者」、鍵バックアップから取り込んだ鍵で読んだものには「送信者を確かめられません」と付けます。
- Web で既存のチャンネルを暗号化した場合、暗号化に切り替える前のメッセージとの境目に「ここから暗号化されています」の区切りを出します。暗号化したメッセージより後ろに暗号化されていないメッセージがあれば、サーバーが足したおそれがあるので「暗号化されていません」と付けます。切り替えの操作は Web で行ってください。
- 一度暗号化チャンネルとして見たチャンネルを、サーバーが暗号化されていないと返したときは、平文では送らずに警告を出します。
- 暗号化した添付はファイル名だけを出し、TUI では開けません。

## 端末と検証（失効、他の人の検証、解錠の手段）

`D` の画面で、Web と Android の設定の「暗号化」と同じことを行えます。`j` `k` で選び、画面の下に出るキーで操作します。

- 端末: 暗号化の鍵を持つ自分の端末を、端末名、登録日時、最後に使った日時、端末 ID で並べます。今の鍵で署名されていない端末には「未署名」と付けます。`n` で端末名を変え、`x` で削除します。端末の削除は失効ではありません（削除した端末は、持っている鍵で今のバックアップを読めます）。なくした端末や見覚えのない端末は、失効から削除してください。端末名は誰でも付けられるので、端末を見分ける根拠にしないでください。
- 失効（`R`）: なくした端末や見覚えのない端末を削除し、鍵を作り直します。次の順に進みます。
  1. 削除する端末を `Space` で選び、漏れた可能性があるもの（`t` で切り替え。迷ったら「パスキー、リカバリーキー、または SSSS の鍵」）を選んで、`Enter` で次へ進みます。
  2. 画面に出る AuthZ の「セキュリティ」と「サービス接続」で、ほかのセッションのログアウトと、zoo の接続の停止を済ませてから `y` を押します。
  3. リカバリーキーで開け直します。全体の失効では新しいリカバリーキーを表示するので、控えてから `y` を押します（古いリカバリーキーとパスキーでは解錠できなくなります。パスキーは失効のあと、Web か Android から足してください）。
  4. 署名し直しとピンの更新についての注意（相手ごとの理由）を確かめ、`Enter` で実行します。ブラウザーでログインし直すと（再認証）、最後まで保存します。終わったら、新しい鍵で署名されていない端末と、AuthZ で行うこと（もう一度の接続の停止とログアウト）を出します。
  - 途中で止まっても、進み具合はこの端末に残ります。もう一度 `D` を開いて `R`（失効を続ける）を押し、リカバリーキーで開け直すと続きから保存します。全体の失効で新しい SSSS を保存したあとなら、新しいリカバリーキーを使います。保存できたか分からないとき（画面に案内が出ます）は、まず失効を始めたときのリカバリーキーを使い、開けなければ新しいほうを試してください。終わっていない失効があるあいだに `zoo2-tui logout` を実行すると、確かめてから消します。
  - 他の自分の端末は、`E` でリカバリーキーで解錠し直すまで鍵を受け取れません。
- 他の人の検証: 暗号化チャンネルを共有する相手の鍵の状態（検証済み、最初に見た鍵を信頼、鍵が変わった、など）を出します。
  - `v`: 絵文字で検証します。相手が受け入れると、互いの画面に絵文字が出るので、相手と見比べて `y`（一致する）か `n`（一致しない）を押します。一致したら、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。
  - 相手から検証を求められると、どの画面にいても前面に出ます（入力の途中なら、ステータス行で知らせるので `D` で開きます）。
  - TUI はカメラを使えないので、相手の QR コードは読み取れません。相手が Web か Android なら、TUI に出る QR コードを相手に読み取ってもらうか、`s` で絵文字を選んでください。
  - `a`: 鍵が変わった相手を再検証します（指紋を相手と読み合わせて、同じときだけ `y`）。できれば絵文字で検証してください。画面の「あなたの鍵の指紋」は、相手があなたを再検証するときに読み合わせる値です。
- 解錠の手段: パスキーとリカバリーキーの鍵スロットを出します。`x` で削除し、`K` でリカバリーキーを作り直します（今のリカバリーキーで開けてから、新しいキーを控えます）。どちらも保存の前にログインし直します。削除と作り直しは失効ではありません。TUI ではパスキーを使えないので、パスキーは Web か Android から足してください。
- 端末が 20 台の上限に達してこの端末を登録できないときは、`D` の画面に上限を出し、30 日以上使っていない端末を選んだ状態にします。`Space` で選び直し、`X` で削除すると、少しあとでこの端末を登録し直します。

## 表示について

- 本文の MFM は、ターミナルで表せる装飾 (太字、斜体、取り消し線、コード、引用、リンク、メンション、ハッシュタグ、絵文字ショートコード) だけを反映します。`$[tada ...]` などの関数は中身の文字だけを表示します。
- 時刻は端末のタイムゾーンで表示します。
- 本文に書いた zoo2 のチャンネル・メッセージへのリンク（接続先の `/channels/<ID>`、`?message=` / `?thread=` 付き）は、行き先が分かると「💬 ○○さん: #チャンネル名」（チャンネルは「#チャンネル名」）に短くし、メッセージの下に投稿者・日時・本文の要約を `┃` の線を付けて出します（本文の URL の先頭 3 件まで）。メッセージを選んで `o` を押すと開き、返信と `?thread=` のリンクはスレッドを開きます。書きかけの返信があるときは、その送り先が変わるリンクは開きません。開いた先のスレッドは、行がそろって実際の親が分かるまで、返信・参加・退出・再開ができません。見られない行き先（存在しない・削除済み・参加していない非公開チャンネル）は「メッセージを表示できません」とだけ出し、開きません。暗号化チャンネルのメッセージは本文を出しません。仕様は [docs/mfm.md](../docs/mfm.md) の「チャンネル・メッセージのリンクの表示」にそろえています。
- 添付ファイル、通知画面、チャンネルの作成と管理には、まだ対応していません。

スレッド一覧は参加チャンネルのアクティブなものを100件ずつ取得します。一覧末尾の「さらに表示」をEnterで選ぶと次の100件を取得します。アーカイブ履歴の一覧・検索画面はありません。

## 設定項目

`config.toml` と環境変数から読み込みます。環境変数が優先されます。

| `config.toml` | 環境変数 | 既定値 | 内容 |
|---|---|---|---|
| `api_base_url` | `ZOO_API_BASE_URL` | `https://zli.chat` | backend の URL。ログインして使う場合は `https://` (`http://` は `localhost` などこの端末の中に限る。`dev_user` を指定したときは制限なし)。ログインの `oidc_issuer` と、そこから分かる接続先も同じ。HTTPS からの `http://` (この端末の外) への転送は追わない |
| `dev_user` | `ZOO_DEV_USER` | なし | dev モードのユーザー。指定したときだけ dev モードで接続し、ログインは使わない |
| `oidc_issuer` | `ZOO_OIDC_ISSUER` | `https://account.zli.works/api/auth` | AuthZ の issuer |
| `oidc_client_id` | `ZOO_OIDC_CLIENT_ID` | 本番の TUI 用クライアントの ID | AuthZ に登録した TUI 用の public client の ID |
| `oidc_resource` | `ZOO_OIDC_RESOURCE` | `https://zli.chat/api` | 要求する resource (RFC 8707)。backend の `OIDC_AUDIENCE` と同じ値にする |
| `oidc_scope` | `ZOO_OIDC_SCOPE` | `openid profile offline_access` | 要求するスコープ |
| `check_updates` | `ZOO_CHECK_UPDATES` | `true` | 起動時に新しい版を調べる。`false` で調べない |

`dev_user` を指定したときだけ dev モードで接続します。指定しなければ OIDC でログインします。

ログインで得たトークンは、そのときの接続先 (`api_base_url`) にだけ送ります。`api_base_url` や OIDC の設定を変えたら、`zoo2-tui login` をやり直してください。

AuthZ には、デバイスコードと refresh_token のグラントを許可した public client を登録しておく必要があります。スコープは `openid profile offline_access`、resource は backend の `OIDC_AUDIENCE` と同じ値です。

## 更新とアンインストール

起動中は、GitHub Releases に新しい版が出ていないかを起動時と6時間おきに調べます。新しい版があれば、画面下のステータス行に「新しい版 vX.Y.Z があります」と出ます。自動では入れ替えないので、下の表の方法で更新してください。

- 問い合わせ先は GitHub の公開 API (`api.github.com`) です。送るのは TUI の版を含む User-Agent だけで、zoo2 のアカウントやトークンは送りません。
- 結果は設定ディレクトリの `update-check.json` に覚え、6時間以内は問い合わせ直しません。
- 調べられなかったとき (オフラインなど) は何も出しません。
- 調べたくないときは `config.toml` に `check_updates = false` を書くか、`ZOO_CHECK_UPDATES=false` を指定します。

| 入れ方 | 更新 | アンインストール |
|---|---|---|
| deb | 新しい `.deb` を落として `sudo apt install ./zoo2-tui_amd64.deb` | `sudo apt remove zoo2-tui` |
| rpm | `sudo dnf install <新しい .rpm の URL>` | `sudo dnf remove zoo2-tui` |
| Arch Linux | 新しい `PKGBUILD` で `makepkg -si` | `sudo pacman -R zoo2-tui-bin` |
| nix profile | `nix profile upgrade zoo2-tui` | `nix profile remove zoo2-tui` |
| NixOS | `nix flake update zoo2` してから再ビルド | 設定から外して再ビルド |
| アーカイブ | 新しいバイナリで置き換える | `sudo rm /usr/local/bin/zoo2-tui` |
| cargo install | 同じコマンドをもう一度実行 | `cargo uninstall zoo2-tui` |

設定とトークンはパッケージを消しても残ります。完全に消すときは `~/.config/zoo2-tui` (macOS では `~/Library/Application Support/zoo2-tui`) を削除してください。消す前に `zoo2-tui logout` を実行すると、IdP 側のトークンも失効し、E2EE の端末もサーバーから削除されます。オフラインなどで端末を削除できなかったときは、`crypto/pending-device-delete.json` に削除待ちが残り、次に同じユーザーで起動したときに消し直します。設定ディレクトリを消すとこの記録も消えるので、先にオンラインで `zoo2-tui run` を一度起動して消し直すか、Web など別の端末の端末一覧から削除してください。

## 困ったとき

- **「ログインしていません」と出る**: `zoo2-tui login` を実行してください。refresh_token が失効したときも同じです。
- **「認証が切れました。再ログインしてください」と出る**: アクセストークンと refresh_token の両方が使えなくなっています。`zoo2-tui login` を実行してログインし直してください。
- **「dev モードではログインできません」と出る**: 環境変数 `ZOO_DEV_USER` か、`config.toml` の `dev_user` が残っています。外してください。
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

`make tui-dev` は `http://localhost:8000` の backend に、既定で `00000000-0000-0000-0000-000000000001` のユーザーとして接続します。別のユーザーで試すときは `ZOO_DEV_USER` を指定します。

```bash
ZOO_DEV_USER=<ユーザーの UUID> make tui-dev
```

### チェック

```bash
make tui-check
```

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` を実行します。テストは backend を使わず、HTTP の応答を wiremock で、画面を ratatui の `TestBackend` で確かめます。

### 構成

通信・認証・状態管理・MFM 構文解析はデスクトップ版と共有するため `../client-core/` に置いています。

- `../client-core/src/config.rs`: 設定の読み込み
- `../client-core/src/http.rs`: 認証ヘッダーとエラー応答
- `../client-core/src/api.rs`: REST API
- `../client-core/src/electric/`: Electric のシェイプ購読と、行の型変換
- `../client-core/src/store.rs`: 行の保持と、画面用の派生ビュー
- `../client-core/src/mfm.rs`: MFM の構文解析
- `../client-core/src/auth/`: デバイスコードログインとトークンの保存・更新、E2EE の再認証（`reauth.rs`。ループバックのリダイレクト）
- `../client-core/src/update.rs`: GitHub Releases の新しい版の確認
- `src/mfm.rs`: MFM 解析結果の ratatui スタイル変換
- `src/e2ee/`: 暗号ストア、端末の登録、初回設定と解錠（`setup.rs`）、暗号化チャンネルの送信（`send.rs`）と送信用セッション・復号（`store/megolm.rs`）
- `src/ui/`: 画面の状態、キー操作、描画（暗号化の画面は `encryption.rs`、暗号化チャンネルの表示と作成の入力欄は `e2ee_chat.rs`）

### リリース

1. `tui/Cargo.toml` の `version` を上げ、`Cargo.lock` を更新して (`cargo check`) PR を出します。
2. main に入ると [CD TUI Release](../.github/workflows/cd-tui-release.yaml) が動き、Linux (musl) と macOS 向けにビルドします。そして deb、rpm、Arch Linux 用の `PKGBUILD`、`SHA256SUMS` を添えて `tui-v<version>` のリリースを作ります。
3. 同じ version のリリースがすでにあるときは何もしません。version を上げずに `tui/` だけ変えても、リリースは作られません。

## ライセンス

zoo2 TUI (`tui/` 配下) は [MIT License](LICENSE) で配布します。

カスタム絵文字のリアクションには `:custom_name:` を入力できる。存在確認とエイリアスの解決はバックエンドが行う。画像の登録・編集は共通絵文字サービスの管理画面で行う。

基盤設定時は `/api/emoji-shortcodes` のgemoji辞書を取得し、本文の表示とリアクション入力に使います。カスタム名・別名を先に解決し、取得失敗時は前回の一覧、成功履歴がなければ同梱辞書を使います。

新規アカウントの自動参加先は、管理者が `zoo2-tui default-channels` で確認し、`zoo2-tui default-channels --set ID1 ID2` で設定できます。ID を省略した `--set` で無効化できます。詳細は [参加チャンネルの設定](../docs/default-channels.md) を参照してください。
