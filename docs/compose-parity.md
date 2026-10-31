# React / Compose Web 対応表

Reactを機能・UI・操作の基準とする。変更時は [compose-react-parity](../.agents/skills/compose-react-parity/SKILL.md) に従い、同じPRで両実装と検証を更新する。
この表は実装の所有先を示すもので、全入力での完全互換や画像の完全一致を保証するものではない。

| 領域 | Reactの主な所有先 | 共通Kotlinの所有先 | 確認先 |
| --- | --- | --- | --- |
| シェル・ルート・レール・ヘッダー | `layout/`、`ChannelHeader` | `App.kt`, `Ui.kt`, `Overlays.kt` | 両実装のentry points / CRUD E2E |
| チャンネル・DM・メンバー・管理 | `ChannelScreen`, `dm`, `WorkspaceMembers` | `Channels.kt`, `Settings.kt` | CRUD・権限境界テスト |
| Zoo設定・検索エンジン・テーマ・言語 | `ZooSettings`, `preferences/`, `theme/`, `i18n/` | `Settings.kt`, `RichText.kt` | Settings / WorkspaceMembersカタログ、設定E2E |
| 投稿・返信・編集・削除・既読 | `messages/MessagePane`, `MessageRow`, `MessageList` | `Chat.kt`, `Repository.kt` | 投稿・返信E2E、スクロール検証 |
| メンション・MFM・チャンネル・コマンド補完 | `messages/Composer`, `composerSuggestions`, `suggestionContext` | `Completion.kt`, `Chat.kt`, `Session.kt` | ParityTest、ParityTransportTest、両実装の補完E2E |
| 本文・MFM・数式・コード | `MfmContent`, `mfm/`, `syntax/` | `RichText.kt`, `MfmEffects.kt`, `Math.kt`, `Syntax.kt` | ContentTest、RichTextカタログ。全構文互換は未確認 |
| 連携カード・Appカードブロック・添付・unfurl | `MessageCard`, `AppBlocks` | `RichText.kt`, `Integrations.kt` | IntegrationCards / AppCardBlockカタログ、カード契約テスト |
| URLカード・メディアプロキシ | `LinkPreviews`, `summaly`, `linkPreviewRequests`, `linkPreviewEndpoints` | `LinkPreviews.kt`, `Chat.kt` | LinkPreviewカタログ、公開HTTPの認証非送信と操作E2E |
| 絵文字・Emoji Kitchen・リアクション | `ReactionPicker`, `emoji/` | `Kitchen.kt`, `RichText.kt`, `Chat.kt` | ContentTest、EmojiPickerカタログ |
| 通知のグループ化・遅延取得・追加読み込み・既読 | `NotificationsScreen`, `notifications/` | `Notifications.kt` | Notifications / NotificationsLoadingカタログ、通知E2E |
| Bubble Chat | `BubbleChatScreen`, `useBubbleChat` | `Bubble.kt` | ParityTransportTest、BubbleChatカタログ、共有・クリア・pagehide E2E |
| App・Webhook・フォーム・権限 | `AppsDialog`, `AppBlocks`, `ChannelWebhooksDialog` | `Integrations.kt`, `Channels.kt` | CRUD E2E、API・ブロック契約テスト |
| API・Electric・保存・再接続 | `lib/api`, `lib/electric`, `lib/offline` | `Repository.kt`, `Session.kt` | RepositoryTest、保存エラー・CSP・オフラインE2E |
| ブラウザー認証・履歴・保存・プッシュ・公開HTTP | `ComposeFrontend`, `lib/compose` | `Platform` / `Main.kt` からホストへ接続、`WebSemantics.kt` でWebのrole・状態を補完 | BrowserInteropTest、OfflineProviderブラウザーテスト、PWA・公開HTTP・ログアウト検証 |
| 翻訳・デザイントークン・アイコン・データ | `lib/i18n`, CSS、Lucide、Emojibase | `Ui.kt`, `IconData.kt`、共有翻訳・生成資源 | 全ロケールの共通辞書、`sync-compose-assets.mjs`、カタログツールバー |

## main取り込み時の照合

- `4983bc3`（URLカード展開時のスクロール保持）: KotlinのLazyColumnでも閲覧中のキー・オフセットを保持し、末尾追従中だけカードの非同期サイズ変更へ追従する。上を読んでいるときの新着はボタンで案内する。
- `19e6517`（通知）: 14spのヘッダー・本文、既読状態、初回の読込表示と追加取得中の既存項目の保持を共通Kotlinへ反映する。
- 設定・Bubbleは共通Kotlinのルートとして描画する。設定タブは選択状態・矢印・Home/End・履歴の置換を扱い、カタログでも実際に切り替える。React画面へのフォールバックで対応済みとはしない。
- `card.cards`、`fields`、Appのcardブロックを扱い、カードを持つ投稿の本文を二重表示しない。Appのファイル添付は既存のダウンロード導線を使う。一般ユーザーのファイルアップロードは現行React版にも存在しないため、架空の未移植機能として記録しない。

## 検証の範囲

Playwrightは共通のHTTPS API fixtureを使い、同じ利用者操作と送信内容を両実装で確認する。
実バックエンド、実OIDC、全Appの失敗・競合、スクリーンリーダー、各OSのIME・タッチ、MFM・数式の全構文、全画面の画像一致は別途確認が必要。
既存の画像比較は不一致を記録する参考ステップであり、必須の機能テストの代用にしない。
