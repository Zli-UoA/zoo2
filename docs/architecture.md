# アプリの構成

## ネイティブ画面と状態

画面は SwiftUI の `NavigationStack`、`List`、`ScrollView`、`sheet` で構成します。
`AppSession` が認証と通知を管理し、認証後に `WorkspaceStore` を作成します。
`WorkspaceStore` はチャンネル、ユーザー、未読、画面遷移、チャンネル別の `ConversationStore` を保持します。
`ConversationStore` は履歴、スレッド、下書き、送信・同期状態を管理します。
UI の状態は `@MainActor` と `@Observable` で管理し、データを `Sendable` の値として扱います。

SwiftUI の `.task(id:)` が画面と前面表示の状態に同期処理を紐付けます。
画面を離れたりバックグラウンドへ移ったりすると通信をキャンセルし、復帰時に再取得します。
ログアウトではワークスペースと下書きを破棄し、遅れて届いた応答を適用しません。
WebView、WebPage、JavaScript ブリッジ、Web 側からの準備完了通知には依存しません。

## 通信

`NativeChatClient` は既存の `APIClient` と認証トークン更新処理を使います。
HTTPS の API リクエストに Bearer トークンを付け、リダイレクトによる別の送信先への転送を拒否します。
読み取りの応答は直接の配列／オブジェクト、書き込みは `{data, txid}` をデコードします。
URL に含める識別子は UUID であることを検証します。

| 操作 | API |
| --- | --- |
| ログイン済みのユーザー | `GET /api/users/me` |
| チャンネル・ユーザー・未読 | `GET /api/channels`、`GET /api/shapes/users`、`GET /api/channel-read-states` |
| 通知の遷移先を確認 | `GET /api/channels/{id}` |
| メッセージ同期 | `GET /api/shapes/messages?channel_id=...` |
| 送信・返信 | `POST /api/messages` |
| 編集・削除 | `PATCH /api/messages/{id}`、`DELETE /api/messages/{id}` |
| スタンプの追加・取り消し | `POST /api/messages/{id}/reactions`、`DELETE /api/messages/{id}/reactions?emoji=...` |
| ワークスペースのスタンプ一覧 | `GET /api/emojis` |
| アカウントスタンプの名簿 | `GET /api/emoji-accounts` |
| 既読 | `POST /api/channels/{id}/read-state` |
| チャンネル・DM の作成 | `POST /api/channels`、`POST /api/dms` |

チャンネル・ユーザー・未読は前面表示中に 30 秒ごとに更新します。
チャンネルが削除されたり閲覧可能な一覧から外れたりしたら、その会話の状態を破棄します。
認可はバックエンドが判断し、同期で 403／404 が返った場合も表示中のメッセージを消します。

REST のユーザー応答には画像 URL が含まれないため、ユーザー一覧は Web と同じ users shape から取得します。
分割応答を `up-to-date` まで集めたスナップショットを使い、`avatar_url` の変更・削除を次の更新で反映します。
アイコンは認証情報を含まない HTTPS URL だけを受け付け、SwiftUI の
[AsyncImage](https://developer.apple.com/documentation/swiftui/asyncimage) で読み込みます。
画像取得には API の Bearer トークンを付けません。未設定・読み込み中・取得失敗時は頭文字を表示します。

## Electric の同期

メッセージと返信のスタンプには Web 版と同じ `display_data.reactions` を使い、外部の利用者を含めて人数を集計します。
この列がない旧形式のメッセージがあるときだけ、チャンネル単位の `/api/shapes/message_reactions` も購読します。
変更の応答を受けたら、自分のスタンプを画面に反映します。
Emoji Kitchen の配信日と組み合わせには Web 版の `frontend/src/lib/emoji/emoji-kitchen.json` を同梱し、対応する合成画像を必要なときに取得します。

初回は `offset=-1` で取得し、`electric-handle`、`electric-offset`、`electric-cursor` を引き継ぎます。
`up-to-date` が届くまで初期取得を続け、その後は `live=true` のロングポーリングへ移ります。
差分の `update` は変更列だけの場合があるため、行のキーごとに既存値へマージします。
`delete` は行を取り除き、論理削除済みのメッセージも表示対象から外します。

本文とモデルのデコードを完了してからカーソルを更新します。
HTTP 409 または `must-refetch` では行を破棄し、キャッシュ回避用の値を付けて再取得します。
通信失敗時は 2 秒から最大 30 秒まで待機時間を増やします。
認証切れでは同期を終了してログイン画面へ戻ります。
初期取得はチャンネル全体が対象です。大量の履歴を持つチャンネルの分割取得は今後の課題です。

## 送信と下書き

通常のメッセージと各スレッドで下書きを分けます。
本文は空白のみを拒否し、バックエンドに合わせ Unicode スカラーで 40,000 文字以内に制限します。
送信ごとに UUID を割り当て、同じ本文を再送するときは同じ UUID を使います。
通信結果が不明な場合でも、バックエンドの冪等性処理で二重投稿を防ぎます。
送信成功後は、その間にユーザーが新しい本文を入力していなければ下書きを消します。

書き込み応答は画面へ即時反映し、Electric が同じ版または新しい版を配信するまで保持します。
古い同期応答で送信済みのメッセージが消えるのを防ぎます。
編集・削除は本人の通常メッセージだけを操作対象にし、最終的な権限検証は API に任せます。
暗号化されたチャンネルへの平文送信は行いません。

## 認証と通知

既存のデバイスコード認証（RFC 8628）と Keychain の形式を維持します。
アプリに表示する確認コードと Safari 認証画面のコードを照合・承認すると、
アプリがトークンを受け取りユーザーを取得します。確認 URL は HTTPS・AuthZ と同じオリジンに限定します。
承認待ち、`slow_down`、HTTP 429、期限切れ、キャンセルへの対処は認証モジュールが担当します。
Associated Domains、AASA、認証コールバックのルートは不要です。

通知設定は SwiftUI の設定画面から操作します。
通知を開くと、ユーザーの一致とチャンネルへのアクセスを確かめてからネイティブ画面へ遷移します。
対象が返信の場合は同期後にそのスレッドを開きます。
複数の通知を続けて開いた場合は新しい遷移先を優先します。
APNs 登録解除の失敗時には既存どおり Keychain の解除情報を保持し、次の接続時に再試行します。

## 検証の境界

単体テストは実際の通信処理にテスト専用 `URLProtocol` を差し込み、UI テストは
Debug 限定の `PreviewChatClient` でネイティブ画面を操作します。
テスト起動時は本番の認証情報・APNs 登録を操作しません。
2026-11-18 に本番のセッション復元、承認されたチャンネルの閲覧、テスト投稿 1 件の送信・編集・再表示を確認しました。
実機の APNs 配信は別途確認が必要です。
暗号化、添付、通話、オフライン永続化などの未対応機能は README に記載します。
