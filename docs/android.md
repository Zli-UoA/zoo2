# Android 版

Android 8.0（API 26）以降を対象にした Compose Multiplatform クライアントです。
Web 版の `frontend-compose/src/commonMain` にある `ZooApp`、画面・部品、
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

Android ターゲットは明示的に有効にします。既存の Web ビルドには Android SDK は不要です。
フォントは Web と同じチェックサム固定の生成処理を使い、翻訳辞書は
`frontend/src/lib/i18n/locales` からビルド時に APK の assets へ取り込みます。
CI Android はビルド・テスト・lint を実行し、debug APK を artifact に保存します。

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

アクセストークンと refresh token はメモリだけで保持し、有効期限の60秒前から更新します。
アプリのプロセス終了後は再ログインが必要です。ログアウトはローカルセッションを終了し、
アカウント別の同期キャッシュを削除します。IdP 全体のブラウザーセッションや他端末の認証は失効させません。
接続設定とアカウント別の下書きはアプリの private preferences に保存し、バックアップは無効にしています。

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

Web と Android の画面実装は共通ですが、タッチ操作・IME・権限別の全画面の実機検証は別途必要です。

## Shizuku による main の自動更新

「アプリの更新」から Shizuku の使用を許可し、「自動更新」を有効にします。
ログイン前でも設定できます。Shizuku は別途インストールし、サービスを起動しておく必要があります。
端末の再起動などで Shizuku が停止した場合は、再び起動してください。
自動更新の有効化だけで Shizuku の権限を自動取得することはありません。

- main の各コミットでテスト・lint 後に固定鍵で署名した release APK をビルドし、
  GitHub Releases の開発版 `android-main` へ配布します。初回配布は main の CI Android を
  `workflow_dispatch` で手動実行しても作成できます。
- APK は `zoo-android-main-<commit SHA>.apk`、更新情報は `update.json` です。
  更新情報には実際のビルドのコミット・バージョン・サイズ・SHA-256 が入ります。
  `android-main` タグ自体は初回作成時のコミットを指す固定の配布チャンネルです。
- バージョンコードは main の first-parent コミット数です。完全な Git 履歴を取得して採番し、
  新しい main が存在する古い実行結果は配布しません。過去のバージョンへ自動で戻しません。
- WorkManager が6時間ごとに、従量制ではないネットワークと十分なバッテリーがあるときに確認します。
  OS のバックグラウンド制限によって実行時刻は遅れることがあります。
  「今すぐ更新を確認・適用」は接続中のネットワークで実行します。
- インストール前にパッケージ名・新しいバージョン・ファイルサイズ・SHA-256・現在の APK と
  同じ署名鍵であることを確認します。APK はアプリ専用キャッシュへ保存し、読み取り用の
  ファイルディスクリプターを Shizuku の UserService へ渡してインストールします。
- 更新によりアプリのプロセスが終了するため、更新後は再起動・再ログインが必要です。
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
