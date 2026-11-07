# React / Compose Web 対応表

Reactを機能・UI・操作の基準とする。変更時は [compose-react-parity](../.agents/skills/compose-react-parity/SKILL.md) に従い、同じPRで両実装と検証を更新する。
この表は実装の所有先を示すもので、全入力での完全互換や画像の完全一致を保証するものではない。

本文の `#チャンネル名` は、Reactでは `MfmChannels.tsx` のルーター `Link`、Composeでは
`RichText.kt` の `LinkAnnotation` → `Session.navigate` → `ComposeFrontend` のルーター履歴で
アプリ内遷移する。Reactの `MfmChannels.browser.test.tsx` はクリック時の通常遷移の抑止、
戻る・進む、シェルの入力保持、Enterキーでの遷移を検証する。既存MfmContentのStoryも検証する。
今回の環境ではComposeの `wasmJsBrowserTest` / `wasmJsBrowserDistribution` はGradle配布物の取得時に
接続を拒否され、再ビルド後のCompose実ブラウザー操作は未検証。

| 領域 | Reactの主な所有先 | 共通Kotlinの所有先 | 確認先 |
| --- | --- | --- | --- |
| シェル・ルート・レール・ヘッダー | `layout/`、`ChannelHeader` | `App.kt`, `PaneHeader.kt`, `Ui.kt`, `Overlays.kt` | 両実装のentry points / CRUD E2E |
| チャンネル・DM・メンバー・管理 | `ChannelScreen`, `dm`, `WorkspaceMembers` | `Channels.kt`, `Settings.kt` | CRUD・権限境界テスト |
| Zoo設定・検索エンジン・テーマ・言語 | `ZooSettings`, `preferences/`, `theme/`, `i18n/` | `Settings.kt`, `RichText.kt` | 既存ZooSettings StoryのKotlin表示、設定E2E |
| 投稿・返信・編集・削除・既読 | `messages/MessagePane`, `MessageRow`, `MessageList` | `Chat.kt`, `Repository.kt` | 投稿・返信E2E、スクロール検証 |
| メンション・MFM・チャンネル・コマンド補完 | `messages/Composer`, `composerSuggestions`, `suggestionContext` | `Completion.kt`, `Chat.kt`, `Session.kt` | ParityTest、ParityTransportTest、両実装の補完E2E |
| 本文・MFM・数式・コード | `MfmContent`, `mfm/`, `syntax/` | `RichText.kt`, `MfmEffects.kt`, `Math.kt`, `Syntax.kt` | ContentTest、RichTextカタログ。全構文互換は未確認 |
| 連携カード・Appカードブロック・添付・unfurl | `MessageCard`, `AppBlocks` | `RichText.kt`, `Integrations.kt` | 既存MessageCard / AppBlocks StoryのKotlin表示、カード契約テスト |
| ファイル添付・画像プレビュー・署名付きURL | `Composer`, `MessageAttachments`, `useAttachments`, `lib/attachments` | `Attachments.kt`, `AnimatedImages.kt`, `Chat.kt`, `Session.kt`, `Repository.kt`、ブラウザープリミティブ `lib/compose/files.ts` | AttachmentsTest、添付の両実装E2E、既存WithAttachments StoryのKotlin表示 |
| URLカード・メディアプロキシ | `LinkPreviews`, `summaly`, `linkPreviewRequests`, `linkPreviewEndpoints` | `LinkPreviews.kt`, `Chat.kt` | 既存LinkPreviews StoryのKotlin表示、公開HTTPの認証非送信と操作E2E |
| 絵文字・Emoji Kitchen・リアクション | `ReactionPicker`, `emoji/` | `Kitchen.kt`, `RichText.kt`, `Chat.kt`, `ReactionChip.kt` | ContentTest、ReactionUsersTest、既存ReactionPicker / ReactionUsers StoryのKotlin表示 |
| 通知のグループ化・遅延取得・追加読み込み・既読 | `NotificationsScreen`, `notifications/` | `Notifications.kt` | 既存Notifications StoryのKotlin表示、通知E2E |
| Home・連合投稿・ソーシャル操作 | `home/HomeScreen`, `HomeComposer`, `HomeReactionPicker`, `ActivityPubEmoji`, `useHomeLive` | `HomeScreen.kt`, `HomeComposer.kt`, `HomeTimeline.kt`, `HomePostCard.kt`, `HomeActivityPub.kt`, `HomeReactionPicker.kt`, `HomeUserSearch.kt`, `HomeLive.kt` | Homeの既存・追加Storyと両実装の操作・画像、Composer状態・設定保存テスト |
| 連合通知・フォロー承認・チャンネル共有設定 | `ActivityPubNotifications`, `ZooSettings` | `ActivityPubNotifications.kt`, `ChannelTimelineSettings.kt` | 既存通知・設定Story、状態/API契約テスト |
| Bubble Chat | `BubbleChatScreen`, `useBubbleChat` | `Bubble.kt` | ParityTransportTest、既存BubbleChat StoryのKotlin表示、共有・クリア・pagehide E2E |
| App・Webhook・フォーム・権限 | `AppsDialog`, `AppBlocks`, `ChannelWebhooksDialog` | `Integrations.kt`, `Channels.kt` | CRUD E2E、API・ブロック契約テスト |
| API・Electric・保存・再接続 | `lib/api`, `lib/electric`, `lib/offline` | `Repository.kt`, `Session.kt` | RepositoryTest、保存エラー・CSP・オフラインE2E |
| ブラウザー認証・履歴・保存・プッシュ・公開HTTP | `ComposeFrontend`, `lib/compose` | `Platform` / `Main.kt` からホストへ接続、`WebSemantics.kt` でWebのrole・状態を補完 | BrowserInteropTest、OfflineProviderブラウザーテスト、PWA・公開HTTP・ログアウト検証 |
| 翻訳・デザイントークン・アイコン・データ | `lib/i18n`, CSS、Lucide、Emojibase | `Ui.kt`, `IconData.kt`、共有翻訳・生成資源 | 全ロケールの共通辞書、`sync-compose-assets.mjs`、カタログツールバー |

## Web描画の性能改善

ARIA補完の更新を対象コントロールに限定し、DOM参照をキャッシュする。アイコンは色別の画像生成をやめ、同じラスタへ描画時に着色する。初回は本文フォントだけをゲートにする。機能・API・操作仕様はReactと共通のまま、Web固有の補完処理を変更する。TUIには対象処理がなく、変更しない。検証方法と結果は [Compose Webの性能計測](compose-performance.md) を参照。

## main取り込み時の照合

- `8815293` の通知保存先共通化と返信ツリー表示を取り込み、通知の表示キャッシュを正規の `notification_events` に接続。通知下部の絵文字一覧は削除し、mainで追加されたアバター上の絵文字は維持する。React／Composeの返信ツリーを維持し、返信数には結合済みデータを優先する。

- 通知カード下部のリアクション絵文字・件数の一覧をReact／Compose双方から削除。通知ラベル・引用本文・移動先を表示する。引用本文中の絵文字は維持する。

- 取得順による投稿・通知のレイアウト変化を抑えるため、`message_display_reactions`・`message_display`・`notification_display` を追加。Electric 1.xはJOINしたVIEWを直接同期できないため、トリガーが同じトランザクションで `messages.display_data`・`notification_events.display_data` へ結果を反映する。投稿者、参照する絵文字と画像寸法、添付、ローカル／連合リアクション、返信数、通知の本文・チャンネル・投稿者をまとめて送る。元のShapeの認可条件を維持し、削除や編集にも追従する。
- Reactは `messageFeedQuery`・`MfmContent`・`MessageList`・`ThreadPane`・`NotificationCard`、Composeは `Chat.kt`・`RichText.kt`・`Notifications.kt` がこの表示データを優先する。旧キャッシュ・楽観更新には従来の補完処理を残す。絵文字のアップロード時に寸法を保存し、既存画像は起動時に補完する。寸法を取得できない画像も固定枠内で縦横比を保って表示する。
- DBに保存していない署名付きURLや外部Summalyの応答はSQL JOINできないため、両実装とも添付画像・URLカードの領域を取得前から確保する。URLカードは横並びの固定高とし、失敗時にも元URLへ移動できる枠を残す。添付画像は単独なら4:3、複数なら正方形の枠内に表示する。
- 確認先: `display_views_test`（旧形式の書き込み、編集・削除、添付、絵文字寸法・別名、連合リアクションとブロック）、`notification_context_test`・`shape_proxy_test`（認可）、Reactの遅延画像・添付・URLカードのブラウザテスト、`ContentTest`・Compose Webのブラウザテスト。`MessageContent/JoinedEmoji` は両実装で同じ横長画像のfixtureを使う。

- `7b0418c` まで取り込み、追加されたAndroid対応を維持しました。今回のHome変更では、Reactの `ActivityPubContent` とKotlinの同名部品から、それぞれチャンネル共通のMFM描画へ接続しています。投稿本文・単独投稿・プロフィール・返信プレビュー・通知本文が対象です。リモート絵文字は既存プロキシを使い、コード内のショートコードは文字列のまま表示します。
- 投稿成功時の公開範囲・連合・同期先と、選択したTLタブの保存をKotlinにも実装しました。アカウント・接続先ごとにキーを分け、既存Platformの保存境界を使います。両実装間で保存データを共有する仕様ではありません。
- 追加したHome／通知のStoryで、PC・390pxのMFM表示と、設定保存・再表示・返信制約・チャンネル無効化・アカウント分離・保存先未解決時の投稿を検証します。HTMLだけで届く連合本文は従来どおりテキストへ変換し、明示的なMisskey MFMの元本文がある場合に装飾を保持します。全MFM構文の互換性や画素単位の一致は保証していません。

- `6514c43` のTUIリリースと `d326387` の開発シェル更新を取り込みました。Runnerテストの競合はTUIのリリース対象とAndroidのHosted登録を両方維持して解消し、28件成功しています。更新情報生成テストも2件成功しています。React／Composeの画面・API変更はありません。

- `794f891` のRunner選択変更を取り込み、Androidのビルド・配布ジョブのHosted登録を維持しました。このコミットにReact／Composeの画面・API変更はありません。Runner構成テスト28件とAndroid更新情報生成テスト2件で確認しています。

- `9dbc553` のSafariキーボード検出修正を取り込みました。Reactの `AppShell` とComposeの `ComposeFrontend` は同じ `useVisualViewport` を使うため、両方に反映されます。キーボード表示・復帰、innerHeightも縮む場合、バウンス時の回帰テストをデスクトップ／モバイルで実行し、6件成功しています。

- 遅延URLカードの拡大後、Reactは初回の未読位置を判定した後、仮想リストの高さをDOMへ反映してから末尾追従を補正する。上へスクロールした状態や編集中は追従を強制しない。Kotlinは既存の `Chat.kt` の測定サイズ監視と `pinned` 判定で同じ挙動を保ち、両実装を `compose.spec.ts` の遅延URLカード検証で確認する。

- `f3784dc` のActivityPubプロフィール設定・投稿/ユーザー詳細ルート・リモートアバター・プロフィールリンク・公開範囲アイコンを共通Kotlinへ反映。Home詳細は `HomeDetail.kt`、設定は `ActivityPubSettings.kt` を使い、既存の投稿カード・通知・API境界と共有する。HomeComposerの閉状態メニュー解放と、モバイル詳細ルートのペイン選択も追従する。追加された12状態を既存Storybookの切替へ登録する。

- `c1efff0` まで取り込み済み。`54cb528` のHome・ActivityPub投稿/返信/リアクション/フォロー/通知・タイムライン設定と追加Storyを共通Kotlinへ反映する。`useVisualViewport` のcallback ref変更もComposeのホスト接続へ反映する。`c1efff0` のTUIは独立クライアントで、WebのUI・API契約に追加変更はない。

- `95657a4` のCIキャッシュ変更とStorybookの非同期検証待ち時間を取り込み済み。React/ComposeのUI・API・Story状態の追加変更はない。

- `a518f22` まで取り込み済み。`d0b9922` のDocker必須ジョブのランナー選択とPreviewの変更検知を維持し、Compose変更もfrontendの変更として検出する。`ebf8e2b` のS3公開エンドポイントは添付APIが返す署名付きURLの変更で、両フロントはURLを組み直さずそのまま取得・ダウンロードする。`a518f22` のマイグレーション検証はフロントの契約・Storyに変更なし。

- `4983bc3`（URLカード展開時のスクロール保持）: KotlinのLazyColumnでも閲覧中のキー・オフセットを保持し、末尾追従中だけカードの非同期サイズ変更へ追従する。上を読んでいるときの新着はボタンで案内する。
- `19e6517`（通知）: 14spのヘッダー・本文、既読状態、初回の読込表示と追加取得中の既存項目の保持を共通Kotlinへ反映する。
- 設定・Bubbleは共通Kotlinのルートとして描画する。設定タブは選択状態・矢印・Home/End・履歴の置換を扱い、カタログでも実際に切り替える。React画面へのフォールバックで対応済みとはしない。
- `card.cards`、`fields`、Appのcardブロックを扱い、カードを持つ投稿の本文を二重表示しない。Appのファイル添付は既存のダウンロード導線を使う。`c82a7b2` で一般ユーザー向けの画像添付が追加されたため、App添付とは別に移植する。

- `c82a7b2`（画像添付）: ファイル選択・貼り付け、10枚・50MiB・画像MIME制約、アップロード・削除・失敗時の復元、添付だけの投稿とスレッド、Electricの添付同期、署名付きURL更新を共通Kotlinへ反映する。GIF・アニメーションWebPはKotlinのブラウザーアダプターで復号し、解放時に再生を止める。ブラウザー接続はファイル・PUT・ダウンロードのプリミティブだけを提供し、React UIは埋め込まない。
- `90e7cb4`（共通ヘッダー）: 36pxのロゴと上下12pxを基準に、共通の60dpヘッダーと1dp区切りへ揃える。チャンネル・一覧・通知・Bubble・設定・スレッド・メンバーパネルで共有する。
- `f319017`（一般ファイル・画像プレビュー）: 画像以外の選択・貼り付け、空ファイル・実行ファイルの拒否、拡張子によるMIME推定、Kotlin製プレビューダイアログの読込・失敗・再試行、Blobによるファイル名付きダウンロードを反映する。

- `6a29bb6`（リアクションしたユーザー）: ユーザー名と表示名、取得中・取得不能の案内、重複排除、PCのホバーとフォーカス時の表示、タッチ・キーボードで開ける一覧を共通Kotlinへ反映する。通常投稿とスレッドで同じ部品を使い、既存ReactionUsers / ReactionUsersHover Storyを上部で切り替えて確認する。

切り替えUIと選択処理は環境変数に依存しない。Compose側のReactに戻る導線は常に表示される。ホストで共有する選択処理を変更した場合は、両実装の往復と保存済み選択を検証する。

## 検証の範囲

Electricの確定スナップショットは、初回・空の結果も保存キューへ渡す。StateFlowの最初の値を捨てる監視には依存せず、保存開始前に届いた初回メッセージもオフラインで復元できるようにする。Wasmのfetchが投げるErrorはHTTP境界でExceptionへ変換し、オフライン時のREST取得失敗がComposeの描画全体を停止させない。キャンセルはそのまま伝播する。SQLite接続の読み書きも接続単位で直列化し、複数Shapeの読み出しが衝突しないようにする。RepositoryTest、接続の並列操作・失敗後の再試行テスト、本番CSP下のオフライン再読み込みで検証する。

Storybookの実装切り替えでは、破棄されたプレビューに届くサイズ監視を無視し、Reactへの復帰時に例外を出さない。既存E2EはCSFのplayによるリアクション一覧表示の完了を待ち、ネイティブの結合されたテキスト、URLカードのlink role、幅変更後の添付プレビューを実際の表示に沿って検証する。

Playwrightは共通のHTTPS API fixtureを使い、同じ利用者操作と送信内容を両実装で確認する。
実バックエンド、実OIDC、全Appの失敗・競合、スクリーンリーダー、各OSのIME・タッチ、MFM・数式の全構文、全画面の画像一致は別途確認が必要。
常用CIはStoryの登録漏れ、Kotlinの契約テスト・ビルド、主要操作のスモーク・API契約、CSP・オフラインを検証する。長時間かかる全StoryのReact/Kotlin描画・操作・画像保存はCIから外し、明示的な依頼がある場合だけ手動で実行する。

Compose WebとStorybookのKotlin生成はGitHubホストのUbuntuランナーを使う。セルフホストでは生成中にランナーとGitHubの通信断が発生したため、この2ジョブは一般用ランナーの選択から分離する。

Storybookは既存Storyの上部implementationツールバーで切り替える。`.storybook/kotlin-stories.ts` と領域別adapterから全380状態へ同じ引数と共有fixtureを渡す。`story-coverage.test.ts` は既存CSFの全exportを照合し、登録漏れを失敗させる。変更範囲の比較には `playwright.storybook-parity.config.ts` と `STORYBOOK_PARITY_FILTER` を使い、同じデータ・時刻・画面幅・テーマ・言語で確認できる。別Featuresカテゴリへ複製しない。

UIの精度は「極端に違わなければ問題ない」という指定に従う。フォントのアンチエイリアスや微小な画素差は移植の完了条件にせず、独自の文字描画エンジンも追加しない。通常の比較は変更範囲に絞り、全件検証を毎回の作業やCIの完了条件にしない。画素比較が必要な明示的な診断だけ `STORYBOOK_PARITY_VISUAL=strict` を指定する。比較ブラウザーは通常のChromiumと同じフォント計測を使う `channel: "chromium"` とし、legacy headless shellの文字幅整数丸めを実装側で再現しない。

## 全件診断で検出された未解決項目

全件診断 run 41303314647（9654ce8）は8バケット中7件が失敗した。全件診断を常用CIから外す判断は実行時間に関する依頼に従うもので、これらの成功や修正完了を意味しない。

- `features-channels-sidebar--unread-and-archived`: Kotlinで未読表示の `img 未読メッセージあり` が見つからない。
- `features-chat-composer--with-file-attachment` と `features-chat-messagecontent--headings`: KotlinのStory完了待ちが90秒でタイムアウトした。描画・playのどちらが原因かは未確定。
- その他のバケットの失敗内容は未確認。全380状態の登録検査の成功は、全状態の描画・操作成功を保証しない。

これらは未解決として残し、対象機能を変更する際に該当Storyに絞って再現・修正する。

## Android での共通実装

Android 版は `ZooApp`・`Session`・`Repository` と既存の画面／状態／API を commonMain から直接使用します。React／Compose Web の画面・API・Story の契約は変更しません。プラットフォーム接続と Android 固有の未対応事項は [Android 版](android.md) に記載します。Android の追加だけを理由に Web の既存 Story を複製しません。

## 未ログインの公開プロフィール

`/public/users/<user-id>` をReact・Compose両方に実装。プロフィールの名前・自己紹介はそれぞれのメイン画面と同じ `ActivityPubProfileDetails` を使い、投稿本文はReactの `HomePostContent`（内部は既存ActivityPubContent/MfmContent）とComposeの既存ActivityPubContentを使う。ボタン・入力欄・アバター・テーマ・翻訳も共通部品を使用する。独自のRust HTML/CSS描画は廃止。

ブラウザでActor URLを開くと公開ルートへ移動し、連携クライアント向けのJSONは維持する。公開APIの20件ページ送り、空・読込・404・失敗・再試行・長文・スマホのStoryを追加。Composeの公開SessionはShape同期・個人用キャッシュ・通知/設定/在席APIを開始しない。公開HTTPはReactのcredentials omitとComposeの匿名ブリッジを使い、認証済み画面へ移動すると公開Runtimeを解放して通常のAuthGateを開始する。

検証済み: フロントの型/lint/formatとビルド、Storybook監査・Kotlin登録検査・ビルド、公開画面のReact Story 8件、共有部分の既存Home Story 6件、公開APIの結合テスト2件、KotlinのRepository/PublicProfile契約テスト8件、Wasm配布用ビルド。公開画面の実ブラウザE2EはReact/Compose各4件で、1280px・390px、匿名のHTTP要求、ページ送り、失敗後の再試行、長文と明暗テーマを確認した。Kotlinの公開プロフィールと既存プロフィールの選択したStoryも実際のCanvasで確認した。

## チャンネル連携投稿と会話ツリー

Homeの投稿詳細は共通の `/api/home/{id}/thread` から、閲覧可能な祖先・対象投稿・返信をツリー順で取得する。Reactの `HomeScreen`、Kotlinの `HomeTimeline` / `HomePostCard` が同じ `tree_depth` を使い、最大6段の字下げとTLでの返信先表示、本文から詳細への移動に対応する。既存の `SinglePost` Storyを祖先・子・孫の4投稿へ拡張した。
チャンネルに紐づく投稿の編集・削除はメッセージAPIの権限検証と保存処理を共有し、Homeの返信は同じチャンネルを引き継ぐ。リアクションはmainのカスタム絵文字・外部リアクションを含む共通実装を維持する。
チャンネルのスレッド内でも返信の子スレッドを開けるよう、Reactの `ThreadPane` とKotlinの `Chat` に同じ操作を追加した。
検証: Reactの型・lint・format、Storybook監査・ビルド、Home/ThreadPaneのStory操作、Kotlin/Wasmのコンパイル・配布ビルド・ブラウザー単体テスト75件、Home・ActivityPub・リアクション・スレッドのAPI統合テスト28件を確認した。会話ツリーと返信先表示は両実装の1440px/390pxで専用Playwrightの4ケースを実行し、字下げ・画面幅・モバイルでのスクロール・返信先リンクを確認した。

## カスタム絵文字

Reactの `ZooSettings`・`CustomEmojiPicker`・`MfmContent`・`ReactionEmoji` と、共通Kotlinの `CustomEmojis.kt`・`Settings.kt`・`RichText.kt`・`Kitchen.kt` を対応させています。両実装から同じ登録・画像アップロード・編集・削除APIを使い、カテゴリ・エイリアス検索、本文・リアクション、非正方形画像を扱います。公開プロフィールはAPIが返すEmojiタグを使います。画像は共通のS3互換バケットの専用prefixに保存し、リモート絵文字の自動カタログ取得は両UIから使われるAPIごと削除しています。

確認対象はZooSettingsのCustomEmojis・Mobile・Empty・Error、リアクション選択、Kotlinのコンパイル・ブラウザテスト、バックエンドの保存・権限・エイリアス解決とMisskeyプロトコルの回帰テストです。実際のMisskeyサーバーとの相互接続試験は実施していません。

検証結果: Reactユニット822件、Kotlinブラウザ74件、ZooSettingsのStory17件、追加4状態のReact/Kotlin操作・表示確認、S3互換ストレージを使う登録・編集・権限テスト、ActivityPub結合16件、通常リアクション結合6件が成功しました。390pxで横方向のはみ出しがないことと、3:1画像の保存・表示、Composeからのファイル選択・アップロードも確認しています。型・lint・Storybook監査、Compose配布ビルド、Storybookビルド、マイグレーション検証が成功しています。画像の完全一致や実Misskeyサーバーとの接続は検証していません。

## チャンネル紐付け投稿の共有リアクション・公開範囲

Reactの `useMessageReactions` とKotlinの `Chat` は、Electricのローカルリアクションへ、権限確認済みAPIの連合リアクションを合流させます。Reactの `useFederatedReactions` とKotlinの `ChannelReactions.kt` はHomeの変更通知で再取得し、画面移動時に購読を解放します。外部アカウント名、同じ絵文字の件数、登録済み／未登録の外部画像の区別を両実装で揃えています。

TLの `HomeScreen` / `HomePostCard` ではチャンネルと共通の `ActivityPubBadge` を日時横に配置します。追加したVisibilityBadges・MobileとFederatedReactions、および既存Federated・CrossPostのStoryを確認対象としています。

今回の検証結果: Reactユニット825件、Kotlinブラウザ75件、Home / MessagePaneのStory65件、追加・変更5状態のReact/Kotlin操作・表示確認、ActivityPub結合17件、通常リアクション結合6件が成功しました。型・lint・format・Storybook監査・ビルド、Compose配布ビルド、マイグレーションの検証に加え、空DBにマイグレーションを再適用したスキーマと `schema.sql` の一致も確認しています。390pxの両実装で日時と公開範囲アイコンの隣接を確認しました。

### PR #311 レビュー対応

React `useFederatedReactions` と共通Kotlin `ChannelReactions` はHome通知をまとめ、初回取得後の再取得を最短5秒間隔に制限する。チャンネル変更・画面破棄では保留処理を停止する。Homeのリアクションチップは両実装ともAPIの `can_react` を反映し、チャンネル紐付け投稿の未登録外部絵文字を無効にする（自分の既存リアクションの取り消しは許可）。単独・外部投稿の受信済み絵文字は同じAPIで再利用する。

共有 `EmojiFallback` Storyは画像失敗時の文字表示と紐付け投稿の無効チップをReact・実Kotlinで検証する。再取得通知の集約はReactの仮想時計テスト、通知元はPostgreSQLの無変更更新／実更新テストで確認する。

検証: Kotlinブラウザ契約75件・Wasm配布ビルド成功。React Home Story60件成功。共有3状態（MisskeyCustomEmoji・MobileMisskey・EmojiFallback）は再生成したKotlin成果物で両実装のplayと描画を検証し、全件成功。画像差分の完全一致はこの検証の対象外。

実アプリのReact・Compose共通E2E全19件も成功。新規の連合リアクションAPIをE2Eモックに追加し、Webhook登録では入力のリセットと一覧反映を順に確認する。

CIの待機処理を調整: 連合リアクションのユーザー一覧はDOM生成後の表示完了まで待つ。Composeのダイアログ契約テストは、再描画でアクセシビリティノードの矩形が一時的になくなる場合、ネイティブのポインター操作を最大5秒再試行する。キャンセル通知・入力リセット・編集保持のアサーションは維持する。

## Homeの画像のみの投稿

Reactの `HomePostContent` は本文がある場合だけMFMを描画し、添付には共通の `MessageAttachments` を使う。Kotlinの `HomePostCard` も通常投稿と返信先プレビューで同じ添付部品を使い、空本文の案内文を挿入しない。Home APIの `attachments` は表示用メタデータだけを返し、画像URLの取得は両実装とも既存の添付APIを使う。
`ImageOnly`・`ImageOnlyDetail`・`ReplyToImageOnly`・`TextWithImage` Storyで本文と添付の組み合わせ、拡大プレビューを確認する。公開プロフィールは空本文の案内文を表示しないが、認証が必要なチャンネル添付の匿名公開は追加しない。
検証: Reactの4 Story、添付・Home Story登録の単体テスト79件、Home/ActivityPub APIテスト22件、Kotlinブラウザーテスト75件と配布ビルドが成功。画像のみ・本文付き・詳細・返信先の4状態を、React/Composeの1440px・390pxで画像の表示・拡大・閉じる操作まで確認した。

## 統合通知一覧

Reactの `useUnifiedNotifications` / `unifiedNotificationFeed` と、Kotlinの `unifiedNotificationFeed` / `Notifications` は、チャンネルとActivityPubを同じ時刻順で扱う。カードは枠・未読状態・アバター右下の実リアクションを共通化し、説明文はアクセシブル名として保持する。前面・フォーカス・可視領域の判定を両方へ適用し、選択既読と全既読を共通操作にした。ローカルの二重通知は一枚にまとめ、両方の記録を更新する。
ActivityPubの追加ページは既読・承認・WebSocket更新で破棄せず保持し、未取得ページより古いチャンネル通知を先に出さない。`UnifiedTimeline` / `UnifiedTimelineMobile` / `UnifiedAutoRead` と既存の承認・拒否・追加読み込みStory、両実装の専用Playwrightを検証先とする。

検証: 通知関連のReact単体テスト67件・Story操作39件、API統合テスト23件、Kotlinブラウザーテスト79件とWasm配布ビルドが成功。React/Composeの1440px・390pxで新着順・アバター右下の通常／カスタム絵文字・選択IDによる自動既読を確認し、Composeの承認・拒否・追加ページ・失敗時操作も4ケース成功。フロントエンドのformat・型・lint、Storybook監査・ビルドも成功。

通知のDB管理は `notification_events` に統合した。既存のReact/Compose用DTO・通知Shapeの8列は保ち、Shapeの参照先と既読APIの保存先を共通化した。クライアントの変更は不要で、両実装の全件既読が同じDB上の状態に反映される。移行・互換テーブル・API契約は [notifications.md](notifications.md) を参照する。


## PR #312 レビュー・CI対応

Reactはスレッドのルートを返信を含む同じメッセージコレクションから解決する。Composeは `?thread=<返信ID>` を開く際にもルートを復元する。両実装のHome返信フォームは保存済みチャンネルを送らず、チャンネル連携投稿でフォロワー限定を選択できない。Composeの投稿カードは本文・時刻に加えて余白からも詳細を開く。

検証先: `compose.spec.ts` の子スレッドへの返信・再読み込みと添付の再試行、`ChannelReplyAudience` Storyの両実装・PC/モバイル、画像のみ投稿の余白クリック。通知の新UIに合わせて3枚のVRT基準画像を実描画を確認して更新し、VRT全19件が成功した。Kotlinブラウザーテスト80件、Home／通知DB／typing APIテスト12件、返信リンクの移行、Atlasのdev DB再生・全スキーマ比較も確認した。

再生成したCompose成果物で、子スレッドへの投稿と再読み込みを両実装で確認した。返信フォーム4ケース、画像のみ投稿の余白クリック2ケース、添付操作2ケース、子スレッド2ケースの計10ケースが成功した。既存のブラウザーテストのモックも新しいルート取得hookに追従させ、全124件が成功した。

- Homeの画像添付はReactの `HomeComposer` / `HomePostContent` とKotlinの `HomeComposer.kt` / `HomeMedia` で対応する。ファイル選択・貼り付け、PNG/JPEG/GIF/WebP、10枚・各8MiB、プレビュー・削除・画像のみ投稿・送信失敗時の下書き保持を共有する。Kotlinは既存の `AttachmentDrafts` と `Platform` のファイル境界を再利用し、アニメーションは `imageFrames` で再生する。チャンネルへの同時投稿は既存の添付表示へ接続し、既存チャンネル投稿の添付もTLへ返す。
- `WithImages` / `MobileImages` / `ImageOnlyPost` を同じ画像fixtureで登録する。検証の件数は以下のPR #315レビュー対応に記載する。追加した3状態の両実装カタログ操作・描画も対象を絞って検証した。画素単位の一致は要件に含めない。
- 画像情報は `home_posts.media` / `apub_remote_posts.media` へ保存する。既存アプリでも書き込める空配列の既定値を持ち、以前のバージョンが受信時に破棄したリモート画像情報は自動復元しない。リモート画像の表示は既存のDNS検証・画像プロキシ・キャッシュ上限を共有する。

- `8815293`（投稿・通知管理の共通化と返信ツリー）へ画像添付の変更をリベースした。Reactの `HomePostContent` とKotlinの `HomePostCard` は、チャンネルの `attachments` がある場合に既存のプレビューを優先し、同じ画像の `media` を重ねて表示しない。本文リンク・返信ツリー・チャンネル編集と通知処理はmainの実装を維持する。両方の画像情報を含むfixtureで二重表示を検証し、検証結果は以下のPR #315レビュー対応に統一する。

## PR #315 レビュー対応

返信元のプレビューにも、チャンネル添付がない場合は `media` を表示する。`ReplyToMediaOnly` はローカル画像・リモート画像だけの返信元を両実装で確認する。画像を選択解除・破棄した場合は未投稿画像のDELETEを試み、通信失敗時はバックエンドの回収処理へ委ねる。投稿成功時には公開した画像を削除しない。リモート写真は専用の8 MiBプロキシを使い、絵文字用の2 MiB制限を保つ。

追加レビューでは `PublicProfile.stories.tsx` の `WithImages` に本文付き・画像のみの公開投稿を登録した。両実装は同じ `imagePublicProfile` と画像APIのfixtureを使い、Reactは画像の表示と復号成功、Composeは画像のアクセシブル名を確認する。画面の実装は既存の `HomePostContent` / `HomeMedia` を引き続き使用する。

追加の3指摘では、アップロードの復号許可をblocking taskの終了時に解放する。S3 PUT待ちの3件が復号用の2枠を占有しないことをAPIテストで確認する。ComposeのHomeは `DraftAttachments` のエラー表示を共通の `home.imageError` に差し替え、8 MiBの案内を一つだけ表示する。`HomeMediaImage` のクリック操作には `Role.Button` を明示する。`ImageUploadError` Storyは両実装でアップロード失敗時のalertが一つで、8 MiBの案内を含むことを確認する。

今回の検証範囲: 対象React Story操作4件、React/Composeの対象6状態の操作・描画、Story登録などの単体テスト76件、Kotlinブラウザーテスト80件と配布ビルドが成功した。Home API11件、Rust単体109件（6件除外）と全ターゲットのclippyも成功した。Storybook監査は423状態を確認した。ReactのCRUD E2EはAPI応答後に編集済みチャンネル名の画面反映も待ち、3回連続で成功した。画素単位の一致は検証対象に含めない。

メッセージ編集欄のCmd/Ctrl+Enter保存（#307）: Reactの `MessageRow` は `shouldSendFromKey` でmacOSはCmd+Enter、Windows / LinuxはCtrl+Enterを判定して保存する。Kotlinの `Chat.kt` の編集欄も `app.env.mac` で同じキーを判定し、保存ボタンと同じ処理を呼ぶ。本文が空白のみ、保存中、IME変換中、Shift+Enterは保存しない。EscapeのキャンセルとEnter単独の改行は変えていない。`frontend/e2e/compose.spec.ts` の投稿・編集フローは、React / Composeの両方で「保存」ボタンの代わりに `ControlOrMeta+Enter` で編集を保存し、編集欄が閉じて本文が更新されることを確認する。Kotlinは `compileKotlinWasmJs` の成功までをローカルで確認している。

## PR #316 レビュー対応

- 表示用の `20261103000200_display_views.sql` と追加の `20261103110000_display_reconciliation.sql` を適用する。既存ファイルを変更せず、追加マイグレーションで再確認キューを更新する。全体フェーズロックやアプリ側の事前取得は使わず、旧経路の `users FOR UPDATE` と投稿が重なってもロック順序を逆転させない。
- 投稿・リアクション・添付・返信数は書き込みと同じトランザクションでVIEWから反映し、コミット後の再確認ジョブも登録する。プロフィール・チャンネル・連合アクター・絵文字の変更は、対象のユーザーや絵文字をキューへ登録する。コミット後に対象行を128件ずつ選び直すため、投稿とメタデータのどちらが先にコミットしても取りこぼさない。
- `display_refresh::worker` は対象を展開するトランザクションと、1行を再計算するトランザクションを分ける。展開は最大128件で投稿行をロックしない。再計算はSKIP LOCKEDと初回200msのタイムアウトで競合を避け、再試行時は最大5秒まで延ばす。ジョブのキーにはトランザクションIDを含め、古いジョブの消費が新しい変更を消さないようにする。メタデータ由来の通知も個別ジョブへ分け、再起動後も処理を続ける。
- 表示項目が変わらないプロフィール更新は何もしない。絵文字の遷移テーブルから変更された名前・別名・画像IDを抽出し、関係する投稿だけを登録する。起動時はS3から寸法を先に取得し、取得成功分を1文で更新する。読み取りは20秒で区切り、起動時の30秒制限で取得成功分まで失わないよう、DBへの反映時間を残す。
- 通知には対象アクターの最新リアクション1件と、本文・バッジに使う絵文字だけを保存する。無関係なリアクションでは通知行を書き直さない。通知下部の一覧は削除した。
- Reactは `useOptimisticDisplay`、Composeは `OptimisticDisplay.kt` でJOIN済みデータに未確定差分だけを重ねる。カスタムリアクションの追加にはピッカーで取得したURL・寸法も重ね、別のHTTP取得を待たずに画像として描画する。ロールバックで差分を除く。Reactの永続化スキーマは4、Composeの保存キーはv2とする。
- Homeのローカル投稿には `message_content_emojis` で本文の名前・別名に対応する絵文字を結合する。Reactの `ActivityPubContent` とComposeの同名コンポーネントは、同じURL・寸法・localフラグを使う。`local: true` の絵文字は外部画像プロキシの表へ登録せず、画像IDの補完前後も直接URLを使う。共有の登録処理も、ローカル記述子・無効なID・相対URLを除く。
- 検証先は `display_views_test` の2万件のメタデータ更新・旧バックエンドのロック順序・独立した投稿/リアクション・キューのSKIP LOCKEDとロールバック、両方のコミット順でのdrift=0、128件の展開上限と世代間のジョブ保持、`display_views.sql` の対象絵文字の絞り込み、`home_test` のHome結合データとID補完後の一覧・詳細取得、`app_event_sources_test` の寸法補完とS3待ちの並行投稿とする。フロントは `optimisticDisplay.test.ts`・`useOptimisticDisplay.browser.test.tsx`・`OptimisticDisplayTest` と、`compose.spec.ts` の両実装での未確定リアクション・返信数・カスタム画像の即時反映と失敗時の復元を確認する。

統合前の旧PR履歴を適用したプレビューDBは、新規DBで検証する。統合後の `d4995b3` のDBには追加マイグレーションを継続適用できることを確認した。旧プレビューは削除済みで実ログを取得できず、失敗原因は未確定のままとする。レビューでは新規DBの適用とCIの成功を確認し、以前のP1は解消と判断されている。本番への適用前には、既存データで初回の投稿・通知キャッシュ構築時間を測る。

### Appへの表示データの非公開と配備順

App向けの messages Shape は取得列を明示し、`metadata:read` の有無にかかわらず `display_data` を返さない。`metadata` 列だけは従来どおり同scopeを必要とする。通常ユーザー向けのShapeは表示キャッシュを引き続き返す。`app.mentioned` と `link.shared` の保存payloadからも表示キャッシュを除き、投稿者プロフィール・リアクションの表示名・添付の内部メタデータがAppへ渡らないようにする。既存DBには `20261103120000_app_message_display_boundary.sql` を追加適用する。

配備はマイグレーション、バックエンド、React・Composeの順に行う。Homeのローカル投稿に絵文字のURL・寸法を返すバックエンドへの切り替えを完了してから、新しいフロントエンドを配信する。旧バックエンドの `emojis: []` は新フロントエンドで確定済みとして扱われるため、この順序を守らないとローカルのカスタム絵文字が一時的に文字列になる。

`20261103000200_display_views.sql` は全投稿・通知の初期キャッシュ構築を一つのトランザクションで行う。ALTER TABLEのロックもコミットまで保持するため、旧バックエンドの稼働中に適用できるスキーマでも、書き込みの無停止は保証しない。本番相当のデータを持つ検証DBで適用時間と投稿・通知へのロック待ちを測り、許容する停止時間に収まることを確認してから適用する。小規模DBでの成功だけを本番の所要時間の根拠にはしない。

### タイムアウト後の再試行とHome通知の本文

`20261103130000_display_refresh_retries.sql` はキューへ `attempts` と `retry_at` を追加する。行・スコープそれぞれの取得に専用インデックスを使い、失敗したジョブの部分更新をロールバックしてから、1秒から最大60秒のバックオフを記録する。ジョブと世代、スコープのカーソルは保持する。実行可能な後続ジョブを先に処理し、スコープの取得自体が失敗しても行ジョブの処理を続ける。ワーカーの実行時間上限は、行が200msから最大5秒、スコープが5秒から最大30秒まで再試行ごとに延ばす。上限でも終わらないジョブは警告を残して再試行し、後続を止めたりジョブを削除したりしない。

Home通知APIも、閲覧可能で削除されていないローカル投稿の本文に `message_content_emojis()` を適用し、名前・別名・URL・寸法・localフラグを `content_emojis` へ返す。Reactの `ActivityPubNotificationCard` とComposeの `ActivityPubNotifications.kt` は既存の同名フィールドを使うため、表示コードの追加は不要。両実装の `LocalContentEmoji` Storyで同じローカル画像・別名を使い、引用本文の画像と縦横比を確認する。TUIはHome通知APIとこの内部キューを使用しないため実装変更は不要で、`make tui-check` を実行する。

今回のローカル検証は、キュー8件・Home API13件・ActivityPub通知3件、42マイグレーションの新規再生・既存41件からの追加適用・schema.sqlとの一致・表示用SQL、全ターゲットのclippy・format、TUI57件が成功した。フロントの型・lint・format、425状態のStory監査・登録契約、対象Storyのplay・a11y、Storybookビルドも成功。Kotlinのブラウザテスト85件と配布ビルドを実行し、生成物を含めたStorybookでReact・Compose各1440px/390pxの4ケースが成功した。画素単位の一致は検証していない。Atlasの関数差分生成はログインが必要で使えず、追加SQLをschema.sqlの定義から構成し、適用結果の列・関数・制約・VIEW・インデックス・トリガーを比較した。

配備前の確認場所とタイミングは次のとおりとする。

- プレビューを再作成・有効化したときは、最新headの `Publish images` 成功後、PRに付く `kustomization/zoo2-pr-316-migrate` の成功と、同じheadのバックエンド起動・投稿・通知取得を確認する。チェックが付かない場合や失敗した場合は、Namespace `zoo2-pr-316` のmigration JobログとFluxの状態を確認し、過去のCI成功だけで配備確認済みとしない。以前の環境の失敗原因は引き続き未確定。
- 本番のmigration Jobを起動する前に、本番スナップショットを復元した隔離PostgreSQL 16で、適用前の履歴から追加マイグレーションを実行する。適用時間に加え、旧バックエンド相当の投稿・通知書き込みを並行実行し、`pg_stat_activity` と `pg_locks` で待ち時間を記録する。投稿・リアクション・通知の件数と結果をPRへ追記し、許容する停止時間を超えた場合は配備を進めず、初期構築の方式を見直す。この本番相当データでの測定はローカル検証に含めない。

### main 851b566 の取り込み

mainのメッセージ編集時のCmd/Ctrl+Enter保存を、Reactの `MessageRow` とComposeの `Chat.kt` の両方に取り込んだ。JOIN済み表示データへの未確定差分の反映は維持する。`compose.spec.ts` の投稿・編集フローと、JOIN済みデータのリアクション・返信数・カスタム絵文字の保留中とロールバックを、再生成したCompose成果物で6ケースを検証し、成功した。バックエンド・DBの内容は、リベース前の検証済みコードと同じである。

TUIは画像やURLカードを描画せず、Shapeの列を明示的に読み取るため、追加の表示用列は既存の読み取りに影響しない。このPRの画像枠・通知カードの変更は追加実装しない。編集時のCtrl+Enterは既存の `on_insert_key` から `submit` を通って保存できる。端末版のEnter送信・Alt/Shift+Enter改行は維持し、`make tui-check` で57テスト・clippy・formatの成功を確認した。

## Androidのログイン復元

#308のResource初期値・段階別の安全な認証エラー表示を取り込み、AndroidはKeystoreで暗号化した更新トークンから起動時に復帰する。接続先をレコードに束縛し、ローテーション・失効・ネットワーク障害・ログアウトと競合する更新を検証する。Webは共通AuthGateの既存OIDC再認可、TUIは既存StoredTokens／TokenManagerによる保存・復元・更新を維持し、別の認証フローを追加しない。Android固有の起動画面と保存境界の変更であり、WebのAPI・表示・操作仕様は変更しない。

Androidのrefresh grantは送信前に保存済みtokenを取り除き、交換・保存をキャンセルから保護する。応答喪失や保存失敗後は手動ログインを要求し、使用済みtokenを再送しない。別のActivity／ログインの保存情報に対する古い処理の書き込み・削除も防ぐ。DNS・接続失敗・接続タイムアウトでは未使用tokenを保存し直して通信復旧後に再試行する。OkHttpの自動再送・リダイレクトは無効にし、送信後の失敗との区別を保つ。Android固有のCoroutine／Keystore保存処理の修正であり、Web／TUIの認証実装には変更しない。

## Compose描画の再計算抑制

チャット一覧・投稿表示データ・本文解析をrememberで保持し、送信ボタンだけがderivedStateOfの送信可否を監視する。編集、同期、楽観更新、添付、絵文字、フォント・テーマの変更では該当するキャッシュを更新する。Web／Androidは同じKotlin実装を使い、Reactの表示・操作・APIは変更しない。TUIにはComposeの再コンポーズ処理がないため、変更しない。比較結果と検証範囲は [Compose Webの性能計測](compose-performance.md#rememberderivedstateofによる再計算抑制) を参照。新しい表示状態は追加しておらず、既存Composer／MessageのStoryを維持し、既存操作のE2Eとライブテーマ変更のCanvas検証を使う。

## 通話の効果音と設定の通話タブ

Compose Web・TUI には音声通話機能（WebRTC、`CallBar`、`call-context`）自体がなく、Reactの通話機能は未移植である。
設定の「通話」タブと入退室の効果音は、この通話機能に付随するため、通話機能を移植するときに合わせて共通Kotlinへ追加する。
通話の移植前にComposeへ効果音のスイッチだけを置くと、効かない設定が表示されるため行わない。Compose側の設定タブは従来のまま
（`tab=call` は一般設定へフォールバックする）で、React側のみの変更である。

## App 管理画面（#322 段階 1）

React に `/apps`（一覧・作成）と `/apps/$appId?section=...`（App 管理画面）を追加し、`AppsDialog` の「管理・開発」タブは設定・認証情報・承認・配信履歴を App 管理画面へのリンクに置き換えた。Webhook の 1 件分の表示・URL 再発行・削除は `ChannelWebhooksDialog` の `WebhookItem` にまとめ、チャンネル設定と App 管理画面で共有する。

Compose Web には移植しない。App 管理画面は React 版 Web だけの機能で、Compose 版は作らない。Compose の `Integrations.kt` は従来どおり App 詳細ダイアログの中で JSON 編集・トークン発行・配信履歴を扱う。新しい管理画面のモジュールは Story を置かず、`frontend/.storybook/coverage.json` でブラウザテストを確認先とする除外として登録した（Story を置くと Kotlin 実装を必須とする `story-coverage.test.ts` が失敗するため）。追加したバックエンド API（App 詳細の `installation`・`updated_at`・`bot_username`、`PATCH` の `expected_updated_at`、`GET /api/apps/{id}/webhooks`、Webhook 作成の `app_id`）は追加のみで、Compose と TUI の既存の呼び出しは変わらない。
