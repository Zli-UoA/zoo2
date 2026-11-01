# React / Compose Web 対応表

Reactを機能・UI・操作の基準とする。変更時は [compose-react-parity](../.agents/skills/compose-react-parity/SKILL.md) に従い、同じPRで両実装と検証を更新する。
この表は実装の所有先を示すもので、全入力での完全互換や画像の完全一致を保証するものではない。

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
| Home・連合投稿・ソーシャル操作 | `home/HomeScreen`, `HomeComposer`, `HomeReactionPicker`, `ActivityPubEmoji`, `useHomeLive` | `HomeScreen.kt`, `HomeComposer.kt`, `HomeTimeline.kt`, `HomePostCard.kt`, `HomeActivityPub.kt`, `HomeReactionPicker.kt`, `HomeUserSearch.kt`, `HomeLive.kt` | 既存Home全47 Storyと両実装の操作・画像、Composer状態テスト |
| 連合通知・フォロー承認・チャンネル共有設定 | `ActivityPubNotifications`, `ZooSettings` | `ActivityPubNotifications.kt`, `ChannelTimelineSettings.kt` | 既存通知・設定Story、状態/API契約テスト |
| Bubble Chat | `BubbleChatScreen`, `useBubbleChat` | `Bubble.kt` | ParityTransportTest、既存BubbleChat StoryのKotlin表示、共有・クリア・pagehide E2E |
| App・Webhook・フォーム・権限 | `AppsDialog`, `AppBlocks`, `ChannelWebhooksDialog` | `Integrations.kt`, `Channels.kt` | CRUD E2E、API・ブロック契約テスト |
| API・Electric・保存・再接続 | `lib/api`, `lib/electric`, `lib/offline` | `Repository.kt`, `Session.kt` | RepositoryTest、保存エラー・CSP・オフラインE2E |
| ブラウザー認証・履歴・保存・プッシュ・公開HTTP | `ComposeFrontend`, `lib/compose` | `Platform` / `Main.kt` からホストへ接続、`WebSemantics.kt` でWebのrole・状態を補完 | BrowserInteropTest、OfflineProviderブラウザーテスト、PWA・公開HTTP・ログアウト検証 |
| 翻訳・デザイントークン・アイコン・データ | `lib/i18n`, CSS、Lucide、Emojibase | `Ui.kt`, `IconData.kt`、共有翻訳・生成資源 | 全ロケールの共通辞書、`sync-compose-assets.mjs`、カタログツールバー |

## main取り込み時の照合

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
