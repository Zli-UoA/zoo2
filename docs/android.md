# Android 版

Android 8.0（API 26）以降を対象にした Compose Multiplatform クライアントです。
Compose for Web の実験は終了しましたが、Android 版の開発・検証・配布は継続します。
共通 Kotlin の `frontend-compose/src/commonMain` にある `ZooApp`、画面・部品、
`Session`、`Repository`、REST API、Electric4k の同期処理をそのまま使います。
Android 用に別のチャット画面や同期クライアントは持ちません。

## ビルド

JDK 21、Android SDK Platform 35、Build Tools 35.0.0 を用意してください。
Android Studio で `frontend-compose` を開き、Gradle プロパティに
`zoo.android=true` を設定するか、コマンドで指定します。
SDK の場所は `ANDROID_HOME` または Git 管理外の `local.properties` で指定します。

```sh
cd frontend-compose
./gradlew -Pzoo.android=true :androidApp:assembleDebug
./gradlew -Pzoo.android=true :androidApp:testDebugUnitTest testDebugUnitTest :androidApp:lintDebug
adb install -r androidApp/build/outputs/apk/debug/androidApp-debug.apk
```

Android ターゲットは `zoo.android=true` で明示的に有効にします。
フォントはチェックサムを固定して取得・生成し、翻訳辞書は
`frontend/src/lib/i18n/locales` からビルド時に APK の assets へ取り込みます。
CI Android はビルド・テスト・lint を実行し、debug APK を artifact に保存します。

## 安定性と端末での確認

### 画面と操作

幅768dp未満では、Home・チャンネル・通知・Bubble Chat・設定をラベル付きの下部ナビゲーションに配置します。
キーボード表示中、スレッドとメンバーの専用画面ではナビゲーションを隠し、入力・返信の領域を確保します。
768dp以上では既存の左ナビゲーションを使い、1024dp以上ではメンバー・スレッドの並列表示を維持します。
通知数は既存の共有状態から取り、ナビゲーション用の追加取得は行いません。

Androidの本文は16sp/24spを基準とし、共通ボタン・入力欄・選択欄・メニューは48dp以上、
チャンネル・DMの一覧行は56dp以上の高さを確保します。ヘッダーとボタンは文字倍率に合わせて高さを広げられます。
設定の説明と操作は配置幅480dp未満で上下に並べます。入力・IME変換・下書き保存は既存の共通処理を使います。

Homeのタイムライン選択には横スクロール可能なタブ、更新にはMaterialのPullToRefreshBoxを使います。
手動更新だけ進行状態を表示し、取得失敗時には既存の投稿を残します。共有の更新集約処理は維持します。
読み込み中・空・取得失敗には共通表示を使い、取得失敗には再試行操作を用意します。
オフライン通知は本文に重ねず、画面内で領域を確保します。
共通部品のラベル・選択・役割・状態通知をAndroidのアクセシビリティツリーにも渡します。

`testDebugUnitTest` はRobolectricのCompose UIテストも実行します。
320dpと600dpのナビゲーション、明暗テーマ、文字倍率180%での日本語入力、タッチ領域と無効状態、
エラーからの再試行を検査します。キャプチャは `frontend-compose/build/reports/ui/` に保存し、
CIの `zoo-android-test-reports` artifactにテストレポートと一緒に収録します。
キャプチャは確認用で、自動の画像差分判定や実機のTalkBack検証ではありません。

### 画像・保存・接続

画像プレビューは受信中に20 MiBの上限を確認し、Content-Lengthがない応答にも適用します。
デコード前に縦横それぞれ2048px以内になるよう縮小し、EXIFの回転・反転を維持します。
これはプレビューの制限で、添付ファイルのアップロード上限（50 MiB）は変更しません。
大きすぎる画像は既存の画像取得失敗表示になります。

同期キャッシュはアカウントごとに原子的に置き換え、読み書きとログアウト時の削除を直列化します。
ログアウト後に届く古いセッションの書き込みではキャッシュを作り直しません。
Androidの戻る履歴には各画面の状態と位置を保存し、メンバー画面から戻る際に前の画面の状態を復元します。
同じパスへの連続移動では戻る手順を増やしません。

ネットワークは既定の接続にINTERNETとVALIDATEDの両方がある場合にオンラインとします。
キャプティブポータルへの接続だけでは、共有Sessionのオンライン復帰による再取得を起動しません。
可視状態はセッション開始時とActivityのライフサイクルから渡し、破棄済みの購読からの遅延通知を無視します。
既存の共有Shape・HomeのWebSocket・ポーリング経路を使い、新しい接続やタイマーは追加しません。

PRの単体テストに加えて、配布前に次を端末で確認します。

- API 26と35で、ログイン・キャンセル・復元・ログアウトを行う。
- ホーム→チャンネル→メンバー→戻る、同じ画面の再選択、ワイド画面への回転で履歴を確認する。
- Wi-Fi切断・キャプティブポータル・通信復帰、バックグラウンドと復帰時の表示と取得回数を確認する。
- 大きな縦長・横長画像、EXIFの回転画像、壊れた画像、20 MiBを超える応答でアプリが操作可能なことを確認する。
- 書き込み中のプロセス終了と再起動、ログアウト直後の再ログインでキャッシュが混ざらないことを確認する。
- 小さい画面・大きい文字・明暗テーマ・TalkBackでログイン入力とエラー通知を確認する。

起動時間・スクロールのフレーム時間・メモリ使用量の性能保証には、実機での計測が別途必要です。
単体テストの成功だけで、これらの性能や端末上の表示品質を保証したとは扱いません。

## ログイン

起動画面で Zoo サーバーの HTTPS origin URL、OIDC issuer、Android 用 public client ID、
resource（backend の `OIDC_AUDIENCE` と同じ値）を入力します。
認証画面をブラウザーで開き、アプリとブラウザーのコードが一致することを確認して承認します。
デバイスコード方式のため callback URI や client secret は不要です。

IdP にデバイス認可（RFC 8628）と refresh token を許可した public client を登録してください。
client ID は自動作成しません。既定の接続先は `https://zli.chat`、
issuer は `https://account.zli.works/api/auth`、Resource / audience は `https://zli.chat/api` です。
AuthZ では Public client の登録に加え、編集画面でデバイスコード認証を許可し、
`openid profile offline_access` と対象 API の利用を許可してください。
Public client であることだけでは、デバイスコード認証は許可されません。

ログイン時の OAuth エラーは HTTP ステータス・エラーコードと確認すべき設定を表示します。
`unauthorized_client` はデバイスコードの許可、`invalid_target` は Resource と API の紐付けを
確認してください。保存済みの接続設定を直す場合は、起動画面で編集してからログインし直します。

初回はブラウザーでコードを承認します。以後はアプリの再起動時に保存した refresh token で
アクセストークンを取得し、同じ接続先へ自動でログインします。更新時に refresh token が
ローテーションした場合も保存し直します。refresh token が発行されないクライアントでは、
従来どおり起動ごとのログインが必要です。

アクセストークンはメモリだけで保持し、有効期限の60秒前から更新します。
refresh token と元の接続設定は、Android Keystore の AES-256-GCM 鍵で暗号化して
private preferences に保存します。平文のトークンは保存せず、バックアップ・端末間転送も無効です。
接続先・issuer・client・Resource は暗号化した同じレコードから復元し、変更した接続先へ
以前のトークンを転送しません。Keystore の鍵が利用できなくなった場合や、トークンが失効した場合は
手動ログインへ戻ります。

認証先の設定取得や DNS・TCP 接続・接続タイムアウトなど、更新リクエストの送信前に通信障害が起きた場合は保存情報を残し、
「自動ログインを再試行」や通信復旧後の更新で復帰できます。OkHttp の自動再送とリダイレクトを無効にし、
送信前の接続失敗と確認できる場合だけ、取り除いた refresh token を保存し直します。保存し直せない場合や
その間にログアウトした場合は復元しません。送信前には古い refresh token を保存領域から取り除き、
取り消しが要求されても、交換結果を受け取って保存するまで待ちます。送信後にタイムアウト・応答喪失・
不正な応答・保存失敗が起きた場合は、古い token を再送せず手動ログインへ戻ります。
AuthZ は使用済み token の再利用を検知すると同じ client・ユーザーの refresh token をすべて失効させるため、
送信後の失敗を単純に再試行することはできません。

復元の待機中も「ログイン画面を開く」で中止を要求できます。交換中の場合は、HTTPタイムアウトを上限に
交換と保存が終わるまで待ってからログイン画面へ戻ります。後から始めた別のログインの保存情報を、
古い処理が上書き・削除することも防ぎます。接続設定を変更して手動ログインを始めると、
以前の保存情報を削除します。ログアウトは保存した認証情報・暗号化鍵・ローカルセッション・
アカウント別の同期キャッシュを削除します。ログアウト後に遅れて届いた更新応答は保存しません。
IdP 全体のブラウザーセッションや他端末の認証は失効させません。
接続設定とアカウント別の下書きは、従来どおりアプリの private preferences に保存します。

## エンドツーエンド暗号化（E2EE）

ログインすると、裏で E2EE の端末を登録し、鍵を補充して、自分の端末あての to-device メッセージ
（Megolm のセッション鍵）を受け取ります（[#359](https://github.com/Zli-UoA/zoo2/issues/359)）。
暗号の処理は zoo-crypto の UniFFI バインディング（`crates/zoo-crypto-ffi`）で行います。
暗号ストアは接続先とユーザーごとに `noBackupFilesDir/e2ee/` に置き、保存鍵は Android Keystore の鍵で包みます。
自動バックアップと端末間の転送には入りません。失敗は表示せず、間隔をあけて試し直します。

ログアウトでは、鍵バックアップに送れていない鍵があれば件数を示して確かめ、暗号ストアを消してから
サーバーの端末を削除します。オフラインなどで削除できなかった端末は、同じユーザーの次のログインで消し直します。
設定の「暗号化」タブで、初回設定と、パスキーかリカバリーキーでの解錠を行います（[#360](https://github.com/Zli-UoA/zoo2/issues/360)）。
初回設定では、リカバリーキーを控えたあと、保存の直前に Custom Tabs でログインし直します（再認証）。
ログインが終わると `https://zli.chat/android/reauth-callback` の App Links でアプリに戻り、同じ画面のまま保存します。
App Links の検証には、zli.chat の nginx が配る `/.well-known/assetlinks.json` が要ります。
frontend のコンテナの `ANDROID_APP_CERT_SHA256` に、配布する APK の署名証明書の SHA-256 の指紋を入れてください
（zoo2-infra。debug APK は署名が違うので、App Links で戻りません）。
AuthZ の Android のクライアントには、認可コードのグラントと、このリダイレクト URI の登録が要ります。
backend は再認証の ID トークンの `aud` を `OIDC_REAUTH_AUDIENCES`（カンマ区切り）で確かめます。この値は zoo2-infra の
production の `backend.yaml` の `env` にあるので、今の値を残したまま Android の client ID を足す PR を zoo2-infra に出してください
（[E2EE の「有効化と運用」](e2ee.md#有効化と運用)）。足すまでは、保存が 403 になります。
リダイレクト先のホストはビルド時の `-Pzoo.android.reauthHost=<ホスト>`（既定は `zli.chat`）で、manifest の App Links と
認可リクエストの両方に使います。接続先のサーバーが違っても、このホストに戻ります。
パスキーは Credential Manager の PRF で使いますが、AuthZ の `assetlinks.json`（AuthZ#96）が要り、実機ではまだ確かめていません。
使えないときは、リカバリーキーで解錠してください。解錠した端末だけが、受け取った鍵を鍵バックアップに送ります。
暗号化チャンネルの送受信、端末一覧と失効、Push の復号は、まだ Android にはありません（#361、#362、#510）。
詳細は [E2EE の「初回設定と解錠の実装」](e2ee.md#compose-android)と[「暗号ストア」](e2ee.md#暗号ストア)を参照してください。

## Android との接続

- `androidApp` は起動、デバイス認証、ナビゲーション履歴、外部ブラウザー、クリップボード、
  ファイル選択と署名付き URL へのアップロード、翻訳、アカウント別キャッシュを接続します。
- `src/androidMain` は文字照合、テキストスタイル、Home の WebSocket の実装です。
- 戻る操作はオーバーレイ、スレッド、画面履歴の順に閉じます。
- システムバーと IME の領域を避け、テーマと表示状態を共有セッションへ渡します。
- 公開画像・URL プレビュー・署名付き添付 URL には Zoo の認証ヘッダーを送りません。
- Web の PWA インストールと Web Push は Android では無効です。バックグラウンドのプッシュ通知、
  外部 URL からのアプリ内 deep link、Android の共有受信には未対応です。
- ファイル選択による添付には対応します。Web 固有のドラッグ＆ドロップとクリップボード画像貼り付け、
  Bubble Chat のタブ離脱通知、アニメーション画像の複数フレーム再生は未対応です。
- Slack 連携の画面は Android にもありますが、「Slack から取り込む」は、ZIP から取り込む方式を廃止したため、Android にはありません。
  管理者の Slack 取り込み画面（「Zoo設定」の「Slack 取り込み」タブ）と、連携ユーザーが非公開チャンネルを取り込む画面は、Web 版だけにあります
  （[Slack からの取り込み](slack-import.md)）。

過去の Web 実験と Android の画面実装は共通ですが、タッチ操作・IME・権限別の全画面の実機検証は別途必要です。

## Shizuku による main の自動更新

「アプリの更新」から Shizuku の使用を許可し、「自動更新」を有効にします。
ログイン前でも設定できます。Shizuku は別途インストールし、サービスを起動しておく必要があります。
端末の再起動などで Shizuku が停止した場合は、再び起動してください。
自動更新の有効化だけで Shizuku の権限を自動取得することはありません。

- main に Android に関係する変更（`frontend-compose/**`、文言、アイコンなど。CI Android の
  `push` の `paths` と同じ）が入るたびに、テスト・lint 後に固定鍵で署名した release APK を
  ビルドし、GitHub Releases の開発版 `android-main` へ配布します。TUI など Android に関係しない
  変更では、ビルドも配布も動きません。初回配布は main の CI Android を
  `workflow_dispatch` で手動実行しても作成できます。
- APK は `zoo-android-main-<commit SHA>.apk`、更新情報は `update.json` です。
  更新情報には実際のビルドのコミット・バージョン・サイズ・SHA-256 が入ります。
  `android-main` タグ自体は初回作成時のコミットを指す固定の配布チャンネルです。
- バージョンコードは main の first-parent コミット数です。完全な Git 履歴を取得して採番し、
  新しい main のコミットに Android に関係する変更が含まれる古い実行結果は配布しません。関係しない
  変更だけが後から入ったときは、同じ内容のビルドとして配布します。過去のバージョンへ自動で戻しません。
- WorkManager が6時間ごとに、従量制ではないネットワークと十分なバッテリーがあるときに確認します。
  OS のバックグラウンド制限によって実行時刻は遅れることがあります。
  「今すぐ更新を確認・適用」は接続中のネットワークで実行します。
- インストール前にパッケージ名・新しいバージョン・ファイルサイズ・SHA-256・現在の APK と
  同じ署名鍵であることを確認します。APK はアプリ専用キャッシュへ保存し、読み取り用の
  ファイルディスクリプターを Shizuku の UserService へ渡してインストールします。
- 更新によりアプリのプロセスが終了するため、更新後は再起動が必要です。保存した認証情報が有効なら自動ログインします。
  アプリのデータとアカウント別の下書きは維持されます。自動更新は初期状態では無効です。
- debug APK は署名が異なるため対象外です。最初に署名付き release APK を導入してください。
  debug 版を削除して入れ替える場合は、そのアプリ内データも削除されます。

### 継続署名の設定

GitHub の Repository Actions Secrets に次の4個を登録します。

| Secret | 内容 |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | 継続して使用する keystore ファイルの Base64（改行なし） |
| `ANDROID_STORE_PASSWORD` | keystore のパスワード |
| `ANDROID_KEY_ALIAS` | 署名鍵の alias |
| `ANDROID_KEY_PASSWORD` | 署名鍵のパスワード |

署名鍵は Git 管理に含めません。鍵一式を安全な場所へ保存し、同じ鍵を使い続けてください。
Secrets がない場合、main の配布ジョブは失敗し、署名の異なる APK を代わりに配布しません。
ローカルで署名する場合は `ZOO_ANDROID_KEYSTORE`（ファイルパス）、
`ZOO_ANDROID_STORE_PASSWORD`、`ZOO_ANDROID_KEY_ALIAS`、`ZOO_ANDROID_KEY_PASSWORD` を
設定し、`-Pzoo.android.versionCode=<整数>` と `-Pzoo.android.versionName=<表示名>` を指定して
`:androidApp:assembleRelease` を実行します。

Shizuku を使用するインストールは、実際に Shizuku が稼働する Android 端末での検証が必要です。

## 自動ログインの検証

認証エラー表示・更新・復元・暗号文の改ざん検出・ログアウトと競合する応答を含む
Androidアプリの単体テスト33件、共通Kotlinテスト、debug APKビルド、Android lintを実行しています。
Android Keystoreを用いた実機／エミュレーターでの起動復帰と、実IdPでのログインは未検証です。

## 共有絵文字サービス

EMOJI_SERVICE_URLを設定したバックエンドからカスタム一覧とgemoji標準辞書を取得し、管理は独立サービスへのリンクへ切り替える。共通KotlinのSessionでカタログ取得を60秒間共有し、明示的な再読み込み・登録/削除後・失敗後の再試行では即時取得する。WorkspaceEmojisTestで並列20呼び出し、登録/削除後の更新、失敗後の再試行を検証する。標準辞書は取得できない間、同梱データへフォールバックする。Web専用ビルド・パリティ資料・Kotlin Storybook登録は復活させない。Reactの管理リンクと辞書は既存Storybook/MSWの実コンポーネントで確認する。
