# zoo2 デスクトップ版 (Tauri)

zoo2 のデスクトップクライアントです。[Tauri v2](https://tauri.app) で作っていて、Linux と macOS を 1 つのコードで扱います。

> **試験版です。** まだ GitHub Releases では配布していません。macOS 版、自動更新、本番の AuthZ へのクライアントの登録は準備中です。下の「[いまの制限](#いまの制限)」も読んでください。

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
| NixOS | ソースから動かします (「[開発](#開発)」) |
| macOS (Apple シリコン) | 準備中です |
| ARM64 の Linux、Intel Mac、Windows | 対応していません |

## インストール

試験版のあいだは、GitHub Actions の「CI Desktop Tauri」が作るファイルを使います。リポジトリを読める GitHub のアカウントが必要です。ファイルは 7 日で消えます。

1. [CI Desktop Tauri の実行一覧](https://github.com/Zli-UoA/zoo2/actions/workflows/ci-desktop-tauri.yaml) で、`main` の成功した実行を開きます。
2. 下の「Artifacts」から `zoo2-desktop-tauri-linux` をダウンロードして、展開します。

[GitHub CLI](https://cli.github.com) を使う場合は、次のようにします。

```bash
gh run download --repo Zli-UoA/zoo2 --name zoo2-desktop-tauri-linux $(gh run list --repo Zli-UoA/zoo2 --workflow ci-desktop-tauri.yaml --branch main --status success --limit 1 --json databaseId -q '.[0].databaseId')
```

中には次の 2 つのファイルが入っています。`0.1.0` の部分は版によって変わります。

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

ログインで得たトークンは、そのときの接続先 (`api_base_url`) にだけ送ります。接続先や OIDC の設定を変えたら、ログインし直してください。

## 更新

いまは、アプリが自分で更新することはありません。新しい版のファイルを「[インストール](#インストール)」と同じ手順で入れ直してください。設定とログインの状態は残ります。

| 入れ方 | 更新 |
|---|---|
| deb | 新しい `.deb` で `sudo apt install ./zoo2_<版>_amd64.deb` |
| AppImage | 新しい `.AppImage` で置き換える |

画面もアプリに入っているので、画面の修正もアプリの更新で届きます。

GitHub Releases での配布と、アプリの中からの自動更新は準備中です (「[いまの制限](#いまの制限)」)。

## アンインストール

| 入れ方 | アンインストール |
|---|---|
| deb | `sudo apt remove zoo2` |
| AppImage | ファイルを消す |

設定とログインの状態は、アプリを消しても残ります。完全に消すときは、先にアプリからログアウトしてから、次のディレクトリを消してください。

- `~/.config/zoo2-desktop-tauri` (設定とトークン)
- `~/.local/share/chat.zli.desktop` と `~/.cache/chat.zli.desktop` (画面のデータとキャッシュ)

## 困ったとき

- **ウィンドウが真っ白、または真っ黒になる**: NVIDIA の GPU と WebKitGTK の組み合わせで起きることがあります。`WEBKIT_DISABLE_DMABUF_RENDERER=1 zoo2-desktop-tauri` で起動してみてください。
- **トレイにアイコンが出ない**: GNOME では、トレイの表示に拡張機能 (AppIndicator / KStatusNotifierItem Support) が必要です。アイコンが出ない間にウィンドウを閉じた場合は、もう一度アプリを起動すると、ウィンドウが戻ります。
- **通知が出ない**: 「[使い方](#使い方)」の、通知を出さない条件に当たっていないか確かめてください。OS の設定で、zoo2 の通知を止めていないかも確かめてください。
- **ログイン画面に戻される**: ログインの期限が切れ、更新もできなくなっています。もう一度ログインしてください。
- **ログインで `invalid_client` や `unauthorized_client` が出る**: AuthZ にデスクトップ版のクライアントが登録されていないか、`oidc_client_id` が違います。管理者に確かめてください。
- **「読み込んでいます…」から進まない**: zli.chat に接続できていません。ネットワークを確かめてください。

## いまの制限

- macOS 版はまだありません。署名と公証をしていないので、配布の準備ができるまでは Linux だけです。
- アプリの中からの自動更新はありません。新しい版は手で入れ直します。
- 本番の AuthZ に `zoo2-desktop-tauri` のクライアントが登録されるまでは、本番の zli.chat にはログインできません。
- 画面は最小限です。スレッド、リアクション、編集、ファイルの添付などは Web 版を使ってください。

## 開発

設計 (同期の境界、IPC、通知の判定、capabilities) は [docs/desktop-tauri.md](../docs/desktop-tauri.md) にあります。開発の環境 (`nix develop`)、`cargo tauri dev` での起動、ローカルの backend での試し方も、そちらの「開発」の節に書いています。
