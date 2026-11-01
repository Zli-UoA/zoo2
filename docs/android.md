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
issuer は `https://account.zli.works/api/auth` です。

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
