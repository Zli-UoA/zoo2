# zoo2 デスクトップ版 (Tauri)

zoo2 のデスクトップクライアントです。[Tauri v2](https://tauri.app) で作っていて、Linux と macOS を 1 つのコードで扱います。

> **試験版です。** 本番の AuthZ へのクライアントの登録は準備中です。macOS 版は、まだ Apple の署名と公証をしていません。下の「[いまの制限](#いまの制限)」も読んでください。

- チャンネル一覧、タイムライン、入力欄を表示します。ほかの人の投稿はすぐに反映されます。
- ウィンドウを閉じても、トレイに残って動き続けます。メンションや DM が届くと、OS の通知を出します。
- ログインは、TUI と同じデバイスコード方式です。アプリに出たコードを、ブラウザーで承認します。

## 目次

- [対応環境](#対応環境)
- [インストール](#インストール)
- [ログイン](#ログイン)
- [使い方](#使い方)
- [設定項目](#設定項目)
- [更新](#更新)
- [アンインストール](#アンインストール)
- [困ったとき](#困ったとき)
- [いまの制限](#いまの制限)
- [開発](#開発)

## 対応環境

| OS | 状況 |
|---|---|
| Ubuntu 22.04 以降、Debian 12 以降 (x86_64) | deb で入れられます |
| そのほかの Linux (x86_64) | AppImage で動きます。WebKitGTK 4.1 が必要です |
| NixOS / Nix (Linux) | flake のパッケージで入れられます (「[NixOS / Nix](#nixos--nix)」) |
| macOS 11 以降 (Apple シリコン、Intel) | dmg で入れられます。署名と公証はまだなので、初回に手順が要ります (「[macOS](#macos)」) |
| ARM64 の Linux、Windows | 対応していません |

## インストール

[GitHub Releases](https://github.com/Zli-UoA/zoo2/releases) の、`zoo2 デスクトップ版 (Tauri)` のリリース (タグは `desktop-tauri-v<版>`) からダウンロードします。一覧の「Latest」は TUI のリリースなので、デスクトップ版は名前で探してください。

[GitHub CLI](https://cli.github.com) を使う場合は、次のようにします。`0.1.0` は入れたい版に置き換えてください。

```bash
gh release download desktop-tauri-v0.1.0 --repo Zli-UoA/zoo2 --pattern 'zoo2_*'
```

リリースから入れると、アプリが自分で更新します (「[更新](#更新)」)。

### 開発中の版

`main` に入ったばかりで、まだリリースしていない版を試すときは、GitHub Actions の「CI Desktop Tauri」が作るファイルを使います。ファイルは 7 日で消えます。この版は自分では更新しません。

1. [CI Desktop Tauri の実行一覧](https://github.com/Zli-UoA/zoo2/actions/workflows/ci-desktop-tauri.yaml) で、`main` の成功した実行を開きます。
2. 下の「Artifacts」から `zoo2-desktop-tauri-linux` をダウンロードして、展開します。

```bash
gh run download --repo Zli-UoA/zoo2 --name zoo2-desktop-tauri-linux $(gh run list --repo Zli-UoA/zoo2 --workflow ci-desktop-tauri.yaml --branch main --status success --limit 1 --json databaseId -q '.[0].databaseId')
```

### ファイル

どちらにも、次の 2 つのファイルがあります。`0.1.0` の部分は版によって変わります。

| ファイル | 対象 |
|---|---|
| `zoo2_0.1.0_amd64.deb` | Ubuntu、Debian |
| `zoo2_0.1.0_amd64.AppImage` | そのほかの Linux |

### Ubuntu / Debian

```bash
sudo apt install ./zoo2_0.1.0_amd64.deb
```

WebKitGTK とトレイのライブラリ (`libwebkit2gtk-4.1-0`、`libayatana-appindicator3-1`) も一緒に入ります。アプリケーションの一覧に「zoo2」が出ます。コマンドは `zoo2-desktop-tauri` です。

### AppImage

```bash
chmod +x zoo2_0.1.0_amd64.AppImage
```

```bash
./zoo2_0.1.0_amd64.AppImage
```

好きな場所に置いて、そのまま起動できます。システムの WebKitGTK 4.1 を使うので、入っていなければ、ディストリビューションのパッケージで入れてください。

### macOS

リリースの `zoo2_<版>_universal.dmg` を開き、`zoo2` を「アプリケーション」に入れます。Apple シリコンと Intel の両方で動きます。

いまの版は Apple の署名と公証をしていないので、そのままでは「開発元を検証できない」などと出て開けません。初回だけ、次のどちらかをしてください。

- ターミナルで、ダウンロードの印 (quarantine) を外します。

  ```bash
  xattr -dr com.apple.quarantine /Applications/zoo2.app
  ```

- 一度開こうとしてから、「システム設定」の「プライバシーとセキュリティ」で「このまま開く」を押します。

2 回目からは、普通に開けます。アプリの中からの更新 (「[更新](#更新)」) も使えます。

### NixOS / Nix

flake の `zoo2-desktop-tauri` パッケージを使います (Linux のみ。x86_64 で確かめています)。どれも 1 行で入れられます。ソースからビルドするので、初回は時間がかかります。WebKitGTK などの依存も一緒に入ります。

インストールせずに起動するときは次のとおりです。

```bash
nix run github:Zli-UoA/zoo2#zoo2-desktop-tauri
```

ユーザーのプロファイルに入れるときは次のとおりです。アプリケーションの一覧に「zoo2」が出ます。

```bash
nix profile install github:Zli-UoA/zoo2#zoo2-desktop-tauri
```

flakes を有効にしていない Nix では、次のようにオプションを付けます。

```bash
nix --extra-experimental-features 'nix-command flakes' profile install github:Zli-UoA/zoo2#zoo2-desktop-tauri
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
          environment.systemPackages = [ zoo2.packages.${pkgs.system}.zoo2-desktop-tauri ];
        })
      ];
    };
  };
}
```

特定のリリースに固定するときは `github:Zli-UoA/zoo2/desktop-tauri-v0.1.0` のようにタグを指定します。

Nix で入れた版は、アプリの中からは更新しません (「[更新](#更新)」)。

## ログイン

1. 起動すると、ログイン画面が出ます。「ブラウザーでログイン」を押します。
2. 画面にコードと URL が出ます。URL をブラウザーで開き、コードが同じことを確かめて承認します。
3. 承認すると、アプリの画面がチャンネル一覧に切り替わります。

ログインの状態は保存されるので、次からはすぐに開きます。ログアウトは、左上の「ログアウト」から行います。

## 使い方

- **チャンネル**: 左の一覧から選びます。Tab で移動し、Enter で開くこともできます。
- **送信**: Enter で送ります。改行は Shift+Enter です。日本語の変換を確定する Enter では送りません。
- **ウィンドウを閉じたとき**: アプリは終わらず、トレイに残ります。トレイのアイコンから「開く」でウィンドウを戻し、「終了」でアプリを終えます。
- **二重起動**: 2 つ目を起動すると、新しく開かずに、動いている方のウィンドウを表示します。
- **通知**: 次のときは通知を出しません。
  - 起動したときや再接続したときに、まとめて届いた過去の通知
  - 自分の操作による通知
  - アカウントに DND (`dnd_until`) が設定されている間。Web 版の Push 通知と同じ扱いで、止めた分を後から出すこともしません (DND はいまのところ App の API から設定します)。DND はログインした直後と 5 分おきに確かめます。ログインした直後に確かめられなかったとき (オフラインなど) は、DND なしとして通知を出し、次に確かめられた時点から止めます
  - ウィンドウが前面にあり、そのチャンネルを表示しているとき
- **通知のクリック**: Linux では、通知をクリックするとそのチャンネルを開きます。

## 設定項目

`~/.config/zoo2-desktop-tauri/config.toml` と環境変数から読み込みます。環境変数が優先されます。ふだんは何も書かなくて構いません。

| `config.toml` | 環境変数 | 既定値 | 内容 |
|---|---|---|---|
| `api_base_url` | `ZOO_API_BASE_URL` | `https://zli.chat` | backend の URL |
| `oidc_issuer` | `ZOO_OIDC_ISSUER` | `https://account.zli.works/api/auth` | AuthZ の issuer |
| `oidc_client_id` | `ZOO_OIDC_CLIENT_ID` | `zoo2-desktop-tauri` | AuthZ に登録したデスクトップ版の public client の ID |
| `oidc_resource` | `ZOO_OIDC_RESOURCE` | `https://zli.chat/api` | 要求する resource。backend の `OIDC_AUDIENCE` と同じ値にする |
| `oidc_scope` | `ZOO_OIDC_SCOPE` | `openid profile offline_access` | 要求するスコープ |
| `check_updates` | `ZOO_CHECK_UPDATES` | `true` | `false` にすると、新しい版を確かめない (「[更新](#更新)」) |
| なし | `ZOO_DESKTOP_UI` | `leptos` | 画面の種類。`remote` にすると zli.chat の Web 版の画面を読み込む (試験中) |

ログインで得たトークンは、そのときの接続先 (`api_base_url`) にだけ送ります。接続先や OIDC の設定を変えたら、ログインし直してください。

## 更新

GitHub Releases から入れたアプリは、自分で新しい版を確かめます。起動の少し後と、その後は 6 時間ごとに確かめます。

1. 新しい版があると、裏でダウンロードし、署名を確かめておきます。
2. 準備ができると、「zoo2 を更新できます」という通知が出て、トレイのメニューに「更新して再起動 (v<版>)」が増えます。
3. 通知かメニューを押すと、新しい版に入れ替えて、アプリを再起動します。押すまでは、いまの版のまま動きます。

| 入れ方 | 入れ替えるとき |
|---|---|
| deb | `dpkg` で入れ直すので、管理者のパスワードを聞かれます。やめた場合は、もう一度押せば入れ直せます |
| AppImage | ファイルを置き換えます。ファイルを置いたディレクトリに書き込めることが必要です |
| macOS | `zoo2.app` を置き換えます。「アプリケーション」に書き込めることが必要です |

設定とログインの状態は残ります。手で入れ直す場合は、新しいファイルを「[インストール](#インストール)」と同じ手順で入れてください (deb は `sudo apt install ./zoo2_<版>_amd64.deb`、AppImage は置き換え)。

CI の成果物から入れた版 (「[開発中の版](#開発中の版)」) と、ソースから動かした場合は、自分では更新しません。

Nix で入れた版も、アプリの中からは更新しません (Nix のストアは書き換えられないため)。Nix の手順で更新します。

| 入れ方 | 更新 |
|---|---|
| `nix profile` | `nix profile upgrade zoo2-desktop-tauri` |
| NixOS の flake | `nix flake update zoo2` してから、`nixos-rebuild switch` (Home Manager なら `home-manager switch`) |

更新を確かめたくないときは、設定の `check_updates = false` (環境変数なら `ZOO_CHECK_UPDATES=false`) にします。TUI と同じ設定です。

画面の種類によって、更新が要る範囲が違います。

- **既定の画面 (`leptos`)**: 画面もアプリに入っているので、画面の修正もアプリの更新で届きます。
- **Web 版の画面 (`remote`)**: 画面は zli.chat から読み込むので、Web 版がデプロイされると、次の起動 (または再読み込み) で新しくなります。ログインや通知など、アプリ側の部分の修正だけがアプリの更新で届きます。アプリと Web 版のやり取りの形 (プロトコルの版) が合わなくなったときは、アプリの更新が必要です。

## アンインストール

| 入れ方 | アンインストール |
|---|---|
| deb | `sudo apt remove zoo2` |
| AppImage | ファイルを消す |
| macOS | 「アプリケーション」の `zoo2.app` をゴミ箱に入れる |
| `nix profile` | `nix profile remove zoo2-desktop-tauri` |
| NixOS の flake | 設定から消して `nixos-rebuild switch` |

設定とログインの状態は、アプリを消しても残ります。完全に消すときは、先にアプリからログアウトしてから、次のディレクトリを消してください。

- `~/.config/zoo2-desktop-tauri` (設定とトークン)
- `~/.local/share/chat.zli.desktop` と `~/.cache/chat.zli.desktop` (画面のデータとキャッシュ)

## 困ったとき

- **ウィンドウが真っ白、または真っ黒になる**: NVIDIA の GPU と WebKitGTK の組み合わせで起きることがあります。`WEBKIT_DISABLE_DMABUF_RENDERER=1 zoo2-desktop-tauri` で起動してみてください。
- **トレイにアイコンが出ない**: GNOME では、トレイの表示に拡張機能 (AppIndicator / KStatusNotifierItem Support) が必要です。アイコンが出ない間にウィンドウを閉じた場合は、もう一度アプリを起動すると、ウィンドウが戻ります。
- **通知が出ない**: 「[使い方](#使い方)」の、通知を出さない条件に当たっていないか確かめてください。OS の設定で、zoo2 の通知を止めていないかも確かめてください。
- **ログイン画面に戻される**: ログインの期限が切れ、更新もできなくなっています。もう一度ログインしてください。
- **ログインで `invalid_client` や `unauthorized_client` が出る**: AuthZ にデスクトップ版のクライアントが登録されていないか、`oidc_client_id` が違います。管理者に確かめてください。
- **「デスクトップ版の更新が必要です」と出る (`remote`)**: Web 版のやり取りの形が、このアプリの版と合っていません。まず「再読み込み」を押し、直らなければ新しい版のアプリを入れてください。
- **更新の通知が出ない**: CI の成果物から入れた版は、自分では更新しません。GitHub Releases の版を入れ直してください。
- **「読み込んでいます…」から進まない**: zli.chat に接続できていません。ネットワークを確かめてください。

## いまの制限

- macOS 版は、Apple の署名と公証をしていません。初回は「[macOS](#macos)」の手順で開きます。通知を押しても、チャンネルは開きません。
- 本番の AuthZ に `zoo2-desktop-tauri` のクライアントが登録されるまでは、本番の zli.chat にはログインできません。
- 画面は最小限です。スレッド、リアクション、編集、ファイルの添付などは Web 版を使ってください。

## 開発

設計 (同期の境界、IPC、通知の判定、capabilities) は [docs/desktop-tauri.md](../docs/desktop-tauri.md) にあります。開発の環境 (`nix develop`)、`cargo tauri dev` での起動、ローカルの backend での試し方も、そちらの「開発」の節に書いています。
