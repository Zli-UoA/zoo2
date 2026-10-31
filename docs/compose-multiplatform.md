# Compose Multiplatform フロントエンド

既存フロントエンドの全機能・同じUIへの移植を目的とした、切り替え可能な実験版です。
既定はReactです。Compose側の画面はKotlinで実装しており、Reactのメッセージ、MFM、数式、絵文字ピッカー、Appフォームを埋め込んでいません。
ただし、全機能・表示の完全互換はまだ確認できていません。以下の検証範囲と制限を確認してください。

## 実行と切り替え

JDK 21、Node、pnpm、初回ビルド時のGradle / Maven / npm / GitHubへの接続が必要です。
WebのビルドにAndroid SDKやXcodeは不要です。

```sh
cd frontend
pnpm install --frozen-lockfile
pnpm build:compose
VITE_EXPERIMENTAL_COMPOSE=true pnpm dev
```

既存のAPI / OIDC設定を使用します。開発用のproxy認証では、既存同様に
`VITE_DEV_USER_ID` を指定してください。サイドバーの切替ボタン、または
`?frontend=compose` で開き、`?frontend=react` で戻せます。選択はブラウザーに保存されます。
`VITE_EXPERIMENTAL_COMPOSE=true` 以外では保存済み選択・URL指定とも無効です。
モジュールの取得・起動に失敗した場合もReactに戻る導線を表示します。
mainで追加された `/settings` と `/bubble` は既存Reactの共通画面を使用します。
Composeの選択は保持し、対応済みのチャンネル画面へ戻るとComposeで表示します。

```sh
cd frontend-compose
./gradlew wasmJsBrowserTest # Chrome/Chromiumが必要。必要に応じてCHROME_BINを指定
./gradlew wasmJsBrowserDistribution
```

インストール済みNodeを使う場合は `-Pzoo.systemNode=true`、`pnpm build:compose` からは
`ZOO_COMPOSE_SYSTEM_NODE=true` を指定できます。
成果物は `frontend-compose/build/dist/wasmJs/productionExecutable/` です。
現在の配布物は約51 MiBで、主にフォントが占めます。初回は読み込み完了を待ち、
以降はService Workerで保存した静的ファイルを再利用します。
`pnpm build:compose` はビルド成功後に `frontend/public/compose/` を置換します。
生成物・依存ソース・フォントバイナリ・GradleキャッシュはGit管理対象外です。
フォントは `frontend-compose/fonts.lock` のURLとSHA-256で固定し、Gradleがビルド時に取得します。
生成先は `frontend-compose/build/generated/composeResources/` です。ライセンス文書は配布物に含めます。

## 実装の配置

`frontend-compose/src/commonMain/kotlin/zoo/` にUI、状態管理、REST、Electric同期を置いています。

| ファイル | 内容 |
| --- | --- |
| `App.kt`, `Ui.kt`, `Overlays.kt`, `NativeStatus.kt` | レール・一覧・レスポンシブ配置、テーマ、フォーム、メニュー、ダイアログ、認証状態画面 |
| `Channels.kt` | チャンネル作成・編集・参加・退出・アーカイブ、メンバー、DM、管理者のメッセージ削除 |
| `Chat.kt`, `Completion.kt` | 投稿・編集・削除、スレッド、リアクション、既読、入力中表示、IME抑制、下書き、メンション・MFM補完 |
| `RichText.kt`, `MfmEffects.kt`, `Math.kt`, `Syntax.kt` | Kotlinによる本文、MFM効果、数式、コードの描画 |
| `Kitchen.kt` | Unicode絵文字、短縮名、検索、最近使用、Emoji Kitchen |
| `Notifications.kt` | 通知・リアクションのグループ化、本文取得、表示時既読、対象メッセージへの遷移 |
| `Integrations.kt` | Appの管理・ホーム・Blocks・フォーム・ショートカット・OAuth同意・配信履歴、Webhook管理 |
| `Repository.kt`, `Session.kt` | Electric4k Shape、REST認証、楽観更新・取り消し、txid待ち、権限・画面状態 |

`wasmJsMain` はブラウザーとの接続です。ホストの `ComposeFrontend.tsx` は、OIDC、ルーター、
認証ヘッダー、OPFS、翻訳カタログ、テーマ・言語、PWA・プッシュ、クリップボードなどを接続します。
これらのブラウザーAPI接続はJavaScriptです。Composeの機能画面をReactで描画する用途では使いません。
非同期処理の接続先は起動したセッションごとに固定し、終了時にHTTP・購読・コルーチンを閉じます。
Compose選択時は既存のTypeScript版Electricコレクションを起動しません。

既存Webと同じフォントファミリー、Lucideのアイコン形状、絵文字データを使用します。
コード用にLiberation Mono、数式用にKaTeXのフォントも同梱しています。
Webでは各フォント・ウェイトの読み込み完了を待ってから画面を表示し、初回の文字欠けを防ぎます。
ライセンスは配布物に含めます。画像はKtorで取得してComposeで描画するため、外部配信元には
ブラウザーからの取得を許可するCORS設定が必要です。

## 配布・オフライン

`infra/Dockerfile-frontend-deploy` は `VITE_EXPERIMENTAL_COMPOSE=true` のときだけJDKステージで
Wasmを生成してWeb配布物へ追加します。既定値は無効です。CDでは本番用の
`VITE_EXPERIMENTAL_COMPOSE_PRODUCTION` とPR用の `VITE_EXPERIMENTAL_COMPOSE_PREVIEW` を別々の
リポジトリ変数として設定します。いずれも既定はfalseです。旧 `VITE_EXPERIMENTAL_COMPOSE` 変数はCDでは参照しません。
ステージングComposeのbuild引数は引き続き `VITE_EXPERIMENTAL_COMPOSE` です。
Nginxは `/compose/` の未存在ファイルに404を返し、Wasm用CSPと圧縮を設定しています。
JavaScriptの `unsafe-eval` は追加していません。Compose用の画像取得元をconnect-srcへ追加するのは
有効なイメージだけで、無効時は既存React版のCSPを維持します。CI Compose Webのcontainerジョブで、
Composeを有効にしたDockerイメージをソースからビルドし、所要時間・配信資源・CSPを確認します。

静的なWasm・フォント・JSはService Worker、ユーザーデータは既存のユーザー別OPFSに保存します。
API・OIDC・SSEレスポンスはService Workerに保存しません。
保存領域の初期化・読み書きに失敗した場合はKotlinのバナーを表示し、再試行できます。
安定した名前のCompose入口はオンラインで検証子を使って更新し、オフライン時に保存済み配布物を使用します。
検証子が同じ場合はCache APIへ書き直さず、HTTPキャッシュにも重複保存しません。
遅れて読み込んだ静的URLだけをService Workerへ通知し、APIやElectricの取得ごとには再送しません。
ログアウト時にはユーザーのOPFS・下書き・プッシュ所有情報を既存の手順で削除します。
ComposeからReactへ戻ってログアウトした場合も共通処理でComposeの下書きを消します。
Previewのユーザー選択もKotlin側に置き、ブラウザーアダプター経由で既存の認証設定を切り替えます。

## 検証と制限

KotlinのChromiumテストは14件です。Shapeの実体化と楽観更新の取り消し、動的認証ヘッダー、
購読のキャンセル、Unicodeのチャンネル名検証、本文・App Blocks・通知・補完・絵文字の正規化を検証します。
本番用Wasmビルド、既存フロントの型・lint・formatチェック、切替テスト、
OfflineProviderのブラウザーテストも実行しています。

```sh
cd frontend
pnpm test:e2e:compose
# 先にComposeとフロントの配布物を生成すること
VITE_EXPERIMENTAL_COMPOSE=true VITE_PREVIEW_AUTH=true VITE_API_BASE_URL='' pnpm build
pnpm exec playwright test --config playwright.compose-production.config.ts
```

E2EはローカルのHTTPS API fixtureを使用します。開発時は同じポートの起動済みサーバーを再利用し、CIでは毎回起動します。投稿・編集・スレッド返信・通知・AppとWebhookの
入口、チャンネル・Webhook・Appの作成と変更、本番相当CSP下での起動、Wasm・フォントの保存、
オフラインでのメッセージ再表示、保存エラーからの再試行を対象にしています。
実際のOIDCプロバイダー、プッシュサービス、Appの接続先、全APIの失敗・競合条件を保証するものではありません。
`compose.spec.ts` の画面比較は既存Reactと同じデータ・時刻・画面サイズで画像を比較します。
UI差分が残る間、この比較を完全互換の証明として扱わないでください。
CIでは機能・CSP・オフライン試験を必須にし、完全一致の画像比較は失敗を記録する追加ステップとして
`continue-on-error: true` で実行し、結果をartifactに保存します。

現在の数式実装は独自のKotlinパーサーで、KaTeXの全命令・マクロとの互換性はありません。
MFMの構文解析・効果とコードハイライトも独自実装であり、既存ライブラリの全入力との同一出力を
保証していません。Appの全状態、全ダイアログ、スクリーンリーダー、各OSのIME・タッチ操作、
外部画像のCORS、既存UIとの全画面一致についても追加の互換性検証が必要です。
PR作成時に最新mainへ追従していますが、mainで追加されたファイル・画像添付とURLカード展開は
Compose側へ未移植です。WebhookのSlack互換受信仕様についても、最新バックエンドを用いた
Compose画面の再検証が必要です。
**「全機能・全く同じUI」の完成条件を満たした版としては扱わないでください。**

最終の検証結果は次のとおりです。

| 検証 | 結果 |
| --- | --- |
| Kotlin/WasmのChromiumテスト | 14件通過 |
| 切替・キャッシュアダプターの単体テスト | 5件通過 |
| React / Composeの主要操作・保存エラー復帰 | 5件通過 |
| 配布物のCSP・オフライン再読み込み | 通過（必要な6フォントの保存完了後に切断） |
| 既存SQLiteのCSP・旧キャッシュ更新 | Firefox / Chromiumの6条件で通過 |
| UIの完全一致比較 | 1440px / 390pxの両方で不一致 |

## Electric4k

Electric4kのソースはZooに同梱せず、サブモジュールも使用しません。
Gradleが公開リポジトリ [Kai-Z-JP/Electric4k](https://github.com/Kai-Z-JP/Electric4k) の固定コミットを
取得し、SHA-256を検証してから、Git管理外の `electric4k/build/upstream/` に展開します。

- コミット: `489e32f85fd608e7e6a883e2f044d5edad8a7c44`
- SHA-256: `94ead69b6bbef7941d4c811a55f2a42dba41123f518e4fff1b268e685f4e399d`

`frontend-compose/electric4k/build.gradle.kts` で両方を管理します。
GitHub認証情報やMaven Localは不要で、初回取得後は通常のGradleキャッシュを利用できます。
現在はupstreamの `commonMain` と `wasmJsMain` を選んでコンパイルします。
LICENSEとNOTICEは配布物の `licenses/electric4k/` に含めます。
更新時はコミットとチェックサム、upstreamの依存ライブラリの設定を一緒に確認してください。

## Desktop / Android / iOS

現在の実行ターゲットはWebのみです。共通UI・REST・同期処理を他ターゲットでも使用する構成ですが、
ターゲット宣言に加えて以下が必要です。

- OSごとの `Platform` 実装：OIDC / PKCE、日時・翻訳、URL遷移、永続化、プッシュ・通知、クリップボード。
- KtorのCIO / OkHttp / Darwinエンジン、各OSの安全なトークン保存とコールバック登録。
- Android SDK、macOS + Xcode、署名・配布設定、各OSのCIと実機試験。

Electric4kの各OS対応だけで、アプリの認証・通知・永続化が揃うわけではありません。

## StorybookのComposeカタログ

既存Storybookの `Features/Compose/Chat` に、実際のKotlinコンポーネントを描画する
Message / GroupedMessage / Composer / DisabledComposer / EmojiPicker / RichText / LongMessageを追加しています。
ReactComparisonでは同じKotlinサンプルの本文で両実装を横に並べます（狭い画面では縦に並びます）。
既存Reactストーリーと同じStorybookで確認でき、テーマ・言語・viewportは既存ツールバー、
本文・グループ化・無効状態はControlsで変更できます。本文が空の場合は共通Kotlinのサンプルを表示します。

```sh
cd frontend
pnpm build:compose  # JDK 21が必要。Kotlinを変更したら再実行
pnpm storybook
```

サンプルとUIは `commonMain/kotlin/zoo/Catalog.kt`、Web起動処理は `wasmJsMain/kotlin/zoo/CatalogMain.kt` です。
Reactのストーリーはiframeを管理してオプションを渡すだけです。iframe内でComposeを起動し、
ストーリーを離れるとCanvas・入力状態・メモリ内の下書きを破棄します。Controls更新は同じiframeに送信します。
APIはKtor MockEngineによるメモリ内の応答で、Electric・認証・OPFS・PWAを起動しません。
入力・送信・絵文字選択はサンプル操作で、実サーバーへの保存を検証するものではありません。
フォント・絵文字データは公開静的ファイルを使用します。Composeのビルドがない場合はカタログ内に案内を表示します。

PRのStorybook PreviewはJDKでComposeをビルドしてから公開するため、レビュー画面でも表示できます。
通常のReact用StorybookテストはJDKやWasm生成を要求しません。Composeストーリーは `!test` タグで
その自動テストから除外し、CI Compose Webで、ビルド済みStorybookに対するPlaywright試験を別途実行します。

```sh
cd frontend
pnpm build-storybook
pnpm exec playwright test --config playwright.compose-storybook.config.ts
```

この試験はKotlinの描画、オプション反映、入力、画面切替時の下書き破棄、モバイル幅の絵文字ピッカーを確認します。
Canvas内部に対する既存DOM用アクセシビリティ検査は同等には使えません。Compose側のsemanticsと操作試験に加え、
スクリーンリーダー等による確認が必要です。カタログ自体はUI完全一致の保証ではありません。
