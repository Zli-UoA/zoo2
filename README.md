# Zoo の iOS アプリ

SwiftUI と Swift で実装した Zoo の iPhone・iPad アプリです。iOS／iPadOS 26 以降に対応し、
チャンネル・DM・スレッドでのチャット、添付ファイル、音声通話、通知を利用できます。
iPhoneと幅の狭いiPadでは「チャンネル」「DM」「通知」「プロフィール」の4タブを使います。
iPadの通常幅では切り替えボタン・会話一覧・本文を並べ、縦・横・上下逆向きの表示に対応します。
REST API と Electric の差分同期で `https://zli.chat` に直接接続します。

この説明は2026-11-21時点のmain（[`02143f7`](https://github.com/Zli-UoA/zoo2-ios/commit/02143f7bc1157fa084fca99855f453c0e3a3d616)）の実装に基づきます。
機能の構成は[構成と通信仕様](docs/architecture.md)、認証・通知の設定は[配備資料](docs/native-auth-push.md)にまとめています。
スクリーンショットと検証結果には、それぞれ記録した時点の日付を記載しています。

## Apple Watchとの連携

watchOS 26以降向けのZooアプリを同梱しています。ペアリングしたiPhoneのZooにログインして使います。
Watchアプリでは通知の種類・送信者を直近10件まで確認でき、着信への「iPhoneで応答」・拒否と、
通話のミュート・終了を操作できます。音声はiPhoneまたはiPhoneに接続した音声機器から流れます。
Watch単体の音声通話と、Watchからの新規発信には対応していません。

メッセージなどの通知は、iPhoneのWatchアプリの「通知」でZooの通知を許可すると、
[OS標準の通知転送](https://developer.apple.com/documentation/watchos-apps/taking-advantage-of-notification-forwarding)で届きます。
通知の表示先はiPhone・Watchのロック状態や集中モードなどに依存します。Watchアプリ内の一覧には
本文を複製せず、通知バナーでの本文表示はOSの通知設定に従います。

通話操作はWatchアプリを開いて、iPhoneと通信できる間に使用します。Watchアプリを閉じた状態での
独自の着信通知や、iPhoneへ接続できない間の操作予約は行いません。着信への応答前に、iPhoneで
マイクの使用を許可してください。接続中の表示はiPhoneの通話状態に追従し、Watchアプリの表示中だけ
15秒ごとに通知一覧を更新します。iPhoneからの表示更新が失敗した場合は、失敗を検知してから
1秒間隔で2回まで、最新の状態と到達性を確認して再送します。その後は通常の更新で回復します。
通信やOSの停止による遅延は保証できません。通信切断・非表示時にはWatch側の表示データを破棄します。

`make generate`でiOSとWatchのプロジェクトを再生成し、`make build`で同梱状態を、
`make build-watch`でWatch単体のビルドを確認できます。回帰テストは`make test`に含まれます。
実機では、通知転送、iPhoneのロック中・バックグラウンドでの応答、音声の送受信、
着信の取消・期限切れ、通信切断と再接続、ログアウト後の表示消去を確認します。

2026-11-21に同梱ビルド・Watch単体ビルド、iOS 26.5のSwift Testing 340件・XCTest 12件、
着信応答・拒否のUIテスト2件が成功しました。reviewer-jpの指摘1件を修正し、再レビューを通過しています。
実機での通知転送・Watchからの通話操作・音声は未検証です。

## スクリーンショット

2026-11-19 に iPhone 18 Pro・iOS 27.0 シミュレーターで撮影した、ライト表示のアプリ画面です。
表示内容はテスト用データで、通話は模擬接続です。画像をクリックすると原寸（1206 × 2622）で開きます。

| チャンネル一覧 | DM一覧 | 通知 |
| :---: | :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/02-channels.png"><img src="artifacts/ios-screenshots-2026-11-19/02-channels.png" alt="チャンネル一覧" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/14-direct-messages.png"><img src="artifacts/ios-screenshots-2026-11-19/14-direct-messages.png" alt="DM一覧" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/15-notifications.png"><img src="artifacts/ios-screenshots-2026-11-19/15-notifications.png" alt="通知" width="230"></a> |

| チャンネルの会話 | スレッド | プロフィール |
| :---: | :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/03-conversation.png"><img src="artifacts/ios-screenshots-2026-11-19/03-conversation.png" alt="チャンネルの会話" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/04-thread.png"><img src="artifacts/ios-screenshots-2026-11-19/04-thread.png" alt="スレッド" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/16-profile.png"><img src="artifacts/ios-screenshots-2026-11-19/16-profile.png" alt="プロフィール" width="230"></a> |

<details>
<summary>ログイン・チャンネル詳細・プロフィール編集・設定</summary>

| ログイン | チャンネル詳細 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/01-login.png"><img src="artifacts/ios-screenshots-2026-11-19/01-login.png" alt="ログイン" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/05-channel-details.png"><img src="artifacts/ios-screenshots-2026-11-19/05-channel-details.png" alt="チャンネル詳細" width="230"></a> |

| 公開プロフィール編集 | 設定 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/17-edit-profile.png"><img src="artifacts/ios-screenshots-2026-11-19/17-edit-profile.png" alt="公開プロフィール編集" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/18-settings.png"><img src="artifacts/ios-screenshots-2026-11-19/18-settings.png" alt="設定" width="230"></a> |

</details>

<details>
<summary>チャンネル検索・作成とDM</summary>

| チャンネルを探す | チャンネル作成 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/10-channel-browser.png"><img src="artifacts/ios-screenshots-2026-11-19/10-channel-browser.png" alt="チャンネルを探す" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/11-create-channel.png"><img src="artifacts/ios-screenshots-2026-11-19/11-create-channel.png" alt="チャンネル作成" width="230"></a> |

| DMの宛先選択 | DMの会話 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/12-dm-recipients.png"><img src="artifacts/ios-screenshots-2026-11-19/12-dm-recipients.png" alt="DMの宛先選択" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/13-direct-message.png"><img src="artifacts/ios-screenshots-2026-11-19/13-direct-message.png" alt="DMの会話" width="230"></a> |

</details>

<details>
<summary>メッセージ操作・スタンプ・編集</summary>

| メッセージ操作 | スタンプ選択 | リアクションした人 |
| :---: | :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/07-message-actions.png"><img src="artifacts/ios-screenshots-2026-11-19/07-message-actions.png" alt="メッセージ操作" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/08-emoji-picker.png"><img src="artifacts/ios-screenshots-2026-11-19/08-emoji-picker.png" alt="スタンプ選択" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/09-reaction-members.png"><img src="artifacts/ios-screenshots-2026-11-19/09-reaction-members.png" alt="リアクションした人" width="230"></a> |

| メッセージの追加メニュー | メッセージ編集 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/27-composer-menu.png"><img src="artifacts/ios-screenshots-2026-11-19/27-composer-menu.png" alt="メッセージの追加メニュー" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/28-edit-message.png"><img src="artifacts/ios-screenshots-2026-11-19/28-edit-message.png" alt="メッセージ編集" width="230"></a> |

</details>

<details>
<summary>画像・PDF・動画</summary>

| 画像プレビュー | PDF添付の会話 | PDFプレビュー |
| :---: | :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/06-image-preview.png"><img src="artifacts/ios-screenshots-2026-11-19/06-image-preview.png" alt="画像プレビュー" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/23-pdf-attachment.png"><img src="artifacts/ios-screenshots-2026-11-19/23-pdf-attachment.png" alt="PDF添付の会話" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/24-pdf-preview.png"><img src="artifacts/ios-screenshots-2026-11-19/24-pdf-preview.png" alt="PDFプレビュー" width="230"></a> |

| 動画添付の会話 | 動画再生 | 動画の全画面再生 |
| :---: | :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/25-video-attachment.png"><img src="artifacts/ios-screenshots-2026-11-19/25-video-attachment.png" alt="動画添付の会話" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/26-video-playback.png"><img src="artifacts/ios-screenshots-2026-11-19/26-video-playback.png" alt="動画再生" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/29-video-fullscreen.png"><img src="artifacts/ios-screenshots-2026-11-19/29-video-fullscreen.png" alt="動画の全画面再生" width="230"></a> |

</details>

<details>
<summary>音声通話・招待・着信</summary>

| 音声通話 | 通話メンバーの招待 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/19-voice-call.png"><img src="artifacts/ios-screenshots-2026-11-19/19-voice-call.png" alt="音声通話" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/20-call-invitation.png"><img src="artifacts/ios-screenshots-2026-11-19/20-call-invitation.png" alt="通話メンバーの招待" width="230"></a> |

| 通話中の会話 | 着信 |
| :---: | :---: |
| <a href="artifacts/ios-screenshots-2026-11-19/21-active-call.png"><img src="artifacts/ios-screenshots-2026-11-19/21-active-call.png" alt="通話中の会話" width="230"></a> | <a href="artifacts/ios-screenshots-2026-11-19/22-incoming-call.png"><img src="artifacts/ios-screenshots-2026-11-19/22-incoming-call.png" alt="着信" width="230"></a> |

</details>

## 技術構成

- 2026-11-21の開発環境はXcode 27.2、Apple Swift 6.4.2、iOS／watchOS 27.2 SDK。最低対応OSはiOS／iPadOS／watchOS 26.0です。
- Swift 6言語モードの厳格な並行性チェックを使用。
- SwiftUI の `NavigationStack`、Observation の `@Observable`、構造化された `async/await`。
- `URLSession` によるHTTPS通信とElectricのSSE差分同期。SSEに接続できない場合はロングポーリングへ切り替えます。
- SwiftDataによる履歴・同期位置の保存と、添付・公開画像のディスクキャッシュ。
- デバイスコード認証、Keychainによるトークン保存、通常通知・VoIP通知の登録と解除、通知サービス拡張。
- WebRTC 157.0.0による音声通話、CallKit・PushKit、WatchConnectivity、SiriKit・App Intents、ActivityKit・WidgetKit。
- Swift Testing の単体・結合テスト、XCTest の画面操作テスト。

チャットAPIの照合記録は2026-11-19の `Zli-UoA/zoo2` main `35cf0be3` です。
VoIP通知の仕様は[認証・通知の配備資料](docs/native-auth-push.md)に参照版を記載しています。
Swift コンパイラーのバージョンと `SWIFT_VERSION: 6.0` の言語モードは別の設定です。

## Siriとショートカット

ログイン後、プロフィールの設定から「SiriでZooを使えるようにする」を選び、Siriの利用を許可します。
「Zooで○○にこんにちはとメッセージを送って」のように宛先と本文を指定する標準の送信要求は、SiriKitの`INSendMessageIntent`で受け付けます。
iOS 27以降にはDMの送信・下書きと音声通話のApp Schemaを登録します。「Zooで○○に電話をかけて」「Zooで○○にこんにちはと送って」のように依頼でき、宛先ごとのショートカット作成は不要です。Siri AIの提供状況・対応端末により利用できる機能は異なります。Siriからのチャンネル投稿・下書き作成には対応しません。
既存のDM・通話のApp Shortcutsと保存済みショートカットの識別子は維持します。宛先を固定したい場合だけ、設定内の「宛先を固定して使う」を参照してください。
宛先は表示名・ユーザー名で検索し、完全一致を優先します。同名候補を勝手に一人へ絞らず、送信先と本文・発信相手を確認してから実行します。通話ではSiri内の確認後にZooを前面に移し、CallKitの通話を開始します。ロック中は認証を求めます。
下書きの作成では送信せず、入力中の異なる下書きも上書きしません。宛先のないDMの下書きは本文を保持してZooの宛先選択画面へ引き継ぎます。Siri AI経由の添付・位置情報・予約送信・ビデオ通話は未対応です。
通話スキーマは[AudioRecordingIntentの要件](https://developer.apple.com/documentation/appintents/audiorecordingintent)に従い、音声開始前にLive Activityを表示します。Zooのライブアクティビティが無効な場合や表示に失敗した場合は発信せず、通話終了・接続失敗・ログアウトで表示も終了します。表示が終了・削除された場合は、その表示に対応する通話を停止します。アプリの再起動時には前回の表示を片付けます。通常のアプリ内発信と既存ショートカットからの発信には、この要件を追加しません。
[Apple公式サンプル](https://developer.apple.com/documentation/appintents/integrating-your-messaging-app-with-apple-intelligence)に合わせ、宛先のUnionValueにはZooの宛先型だけを登録し、標準連絡先はIntentValueQueryで変換します。Siri由来の別形式の識別子はユーザー名・表示名の完全一致で照合し、Zooのアカウント付き識別子は別アカウントへ引き継ぎません。同名候補を勝手に一人へ絞ることもありません。
メンバー候補には現在のアカウントで同期済みの一覧を使い、未取得の場合だけ通信で取得します。実行直前にも相手の有効性をサーバーで再検証します。Siriの利用を許可すると宛先を検索用に登録し、ログアウトや許可の取り消し時には削除します。
DMの送信・通話は1対1が対象です。無効なメンバー・bot・自分宛ては候補に含めず、暗号化DMへのメッセージ送信には対応しません。別アカウントへ切り替えた場合は、保存済みショートカットの宛先を選び直してください。

[App Intents](https://developer.apple.com/documentation/appintents)とApp Shortcutsに加え、[SiriKitのメッセージ送信](https://developer.apple.com/documentation/sirikit/messaging)を登録しています。自動テストでは標準送信ハンドラー・宛先検証・REST送信・通話開始と中断を確認します。Siriの音声認識と確認ダイアログ、実際の音声通話は実機で確認してください。

## 開発手順

Xcode で `Zoo.xcodeproj` を開きます。プロジェクトは XcodeGen で `project.yml` から生成します。

```sh
make generate
make build
make build-watch
make test
```

`make build`はWatchアプリと通知・通話表示の拡張を含めてビルドします。
`make build-watch`はWatchアプリだけをビルドします。
`make test`は`ZooTests`と`ZooUITests`を実行し、Watch連携のロジックもiOS側で検証します。
`DESTINATION`でiOSシミュレーター、`WATCH_DESTINATION`でWatchのビルド先を指定できます。
利用できる端末名は`xcrun simctl list devices available`で確認してください。

```sh
make test DESTINATION='platform=iOS Simulator,name=iPhone 18 Pro'
```

実機向けの署名には、チーム `F82XB5KFX2` の App ID `works.zli.zoo` と
Push Notifications の機能設定が必要です。Debug は APNs のサンドボックス環境、Release は本番環境を使います。
デバイスコード認証に Associated Domains や AASA は不要です。

## 利用できる機能

- ログイン、保存済みセッションの復元、トークン更新、ログアウト。
- 「チャンネル」「DM」「通知」「プロフィール」の4タブで移動できます。チャンネルとDMは別々に開いている画面と下書きを保持します。
- 参加中のチャンネルとスレッドの一覧、未読の絞り込み、公開チャンネルの検索・参加、公開／非公開チャンネルの作成。DMは1対1とグループに対応し、宛先を9人まで選べます。
- メッセージの閲覧・差分同期・送信・編集・削除、スレッドの閲覧と返信。参加行があるスレッドへ未参加・退出済みのまま返信して403になった場合は、スレッドへ参加（アーカイブ済みは先に再Open）し、同じIDで一度だけ再送します。参加不足以外の403（投稿権限なしなど）や、参加できない場合は、参加せず下書きを保持してエラーを表示します。
- チャンネル・DM・スレッドの入力欄の上に「さくらが入力中…」を表示します。iOSからも入力状態を通知し、入力停止・送信・画面離脱時に解除します。本文や下書きは入力中通知に含めません。
- Web版のピン留めを「ピン留め」にまとめ、通常の「一覧」には重複表示しません。一覧のチャンネルを長押しするか、チャンネル詳細の「会話について」にあるピン留めトグルで追加・解除できます。会話上部のチャンネル名をタップすると詳細を開き、説明・公開範囲・メンバーを確認できます。
- 管理権限のあるチャンネルでは、詳細画面から名前・説明の編集とアーカイブができます。
- 日付の区切りと、会話を開いた時点の「ここから未読」を表示します。既読になっても、その訪問中の境界は残ります。
- メンションを色分けし、入力中の `@` から参加メンバー・`@channel`・`@here` を選べます。
- 添付画像を表示・拡大できます。センシティブな添付は明示的に開くまで隠します。
- 動画は先頭フレームと中央の再生ボタンを表示し、チャンネル・DM・スレッド内でタップして再生できます。iOS標準の再生・シーク・全画面・ピクチャインピクチャ（PiP）に対応し、PiPから元のメッセージへ戻れます。音声通話を開始すると動画を停止します。
- 添付PDFはチャット内に1ページ目をプレビュー表示します。タップすると全画面で開き、複数ページの縦スクロールやピンチ操作での拡大縮小ができます。読み込み失敗時は再試行できます。パスワード付きPDFは未対応です。
- 取得済みの添付画像・PDFは端末にキャッシュし、再表示・拡大・保存で再利用します。オフラインでも開けますが、容量上限の256MBを超えた場合やOSによって削除された場合は再取得が必要です。
- 画像・PDFの詳細は、下へ大きく引くか速くスワイプすると閉じます。画像は全体表示中、PDFは拡大していない状態で先頭から引いた場合に有効です。軽いドラッグ、拡大中の移動、PDFの途中ページから戻るスクロールでは閉じません。
- 入力欄の「＋」→「写真・動画を添付」から写真ライブラリを、「ファイルを添付」から端末やiCloud Driveのファイルを選択できます。チャンネル・DM・スレッド返信に合計10個、各50MBまで添付でき、本文なしでも送信できます。送信前の削除と、失敗後の添付を保持した再送に対応します。
- 添付ごとにセンシティブ指定を選べます。HEIC／HEIF画像は取り込み時にJPEGへ変換し、変換後もサイズ上限を確認します。
- 付いているスタンプを長押しすると、リアクションした人を確認できます。
- チャンネル・DMで最大6人の音声通話に参加でき、ミュート・スピーカー切替・退出ができます。CallKitの標準着信画面で応答・拒否でき、アプリ内でも同じ着信音が鳴ります。アプリ内の着信表示には「参加」「拒否」があります。ロック中・バックグラウンドの着信にはサーバーのVoIP対応と通知設定の有効化が必要です。
- チャンネル通話の「メンバーを招待」から、空き人数までメンバーを選んで呼び出せます。呼び出し中の表示・取り消し、送信失敗後の再試行に対応します。DMは通話開始時の自動呼び出しを使います。
- 本文中の HTTP／HTTPS URL をタップして開けます。`:名前:` はカスタムスタンプ（別名を含む）または標準絵文字として表示し、`:@ユーザー名:` はアカウントの画像を表示します。未登録名・画像の取得失敗時・バッククォートで囲んだコードは原文を残し、コピー・編集には入力した本文を使います。
- メッセージを長押しして「返信する」を選ぶか、参加者アイコン・返信件数・最終返信日時をまとめたリンクをタップすると、スレッドを開けます。見出しに元の会話名を表示し、親投稿と返信一覧を件数付きの線で区切ります。返信がない場合は案内を表示し、短いスレッドは上から読める配置にします。戻るボタンや画面左端からのスワイプで元の会話に戻れます。スタンプは長押しメニューの候補や顔の「＋」から追加でき、メッセージの下に付いているスタンプと人数を表示します。
- 長押しのスタンプ候補は直近に追加した順です。利用履歴を端末内に16件まで保存し、サーバーとアカウントごとに分けます。候補6件の不足分は既存のスタンプと既定候補で補い、追加失敗や取り消しでは履歴を変更しません。
- Webと同じユーザーアイコンをチャット・スレッド・DM・アカウントに表示します。アイコンとスタンプ画像は端末に保存して再表示に使い、アイコンの更新も取得します。未設定・未取得時は頭文字で代替し、取得済みの画像は更新に失敗しても表示を続けます。
- チャンネルやスレッドごとの下書き保持。送信に失敗した場合も本文を残し、同じ ID で再送。
- 通知タブはチャットと連合の通知をまとめ、対象のメッセージや投稿を開けます。未読バッジ、表示した通知の既読、全件既読、フォロー申請の承認・拒否に対応します。プッシュ通知の受信許可は任意です。
- プロフィールから公開表示名・自己紹介を編集できます。設定には配色・プッシュ通知・Siri連携・ログアウトがあり、名前やアイコンの変更にはアカウント管理画面を開きます。
- フォアグラウンド復帰時の再接続。バックグラウンドでは画面に紐付いた同期を停止。
- メッセージ・一覧・ユーザー・通知と同期位置を SwiftData に保存し、オフラインでは取得済みの内容を表示します。再接続時は保存した位置から差分を取得します。
- 会話とスレッドは直近50件から表示し、上へスクロールして過去の部分に近づくと50件ずつ自動で追加します。

メッセージの全文検索、暗号化チャンネルの復号・送信、
アプリのメッセージ部品、オフラインでの書き込み予約は未対応です。
暗号化チャンネルとアーカイブ済みチャンネルでは送信を無効にします。
発信・着信ともCallKitへ登録し、標準通話画面から終了・ミュートできます。Siriからの発信と再接続も同じ経路を使い、再接続ではミュート状態を標準画面にも引き継ぎます。1人待機中と相手との接続交渉中は、CallKitが有効化した音声セッションで音声入力を維持し、WebRTCの音声入出力開始時に引き継ぎます。待機中の入力は保存・送信しません。最後の相手が退出すると待機入力へ戻り、通話終了・音声セッションの無効化で停止します。拒否・取消・期限切れ・ログアウト時には着信を終了します。音量・消音・集中モードはiOSの設定に従います。PushKitの受信処理は画面表示前に初期化し、通常通知とVoIP通知の端末登録・解除を分離しています。
チャンネル一覧・ユーザー・未読は起動時と前面表示中の 30 秒ごとに更新し、
選択中のチャンネルのメッセージは Electric で継続的に同期します。
差分の受信には SSE を使い、接続できない場合はロングポーリングに切り替えます。書き込みと未読情報の取得は REST API を使います。
同期データと添付のキャッシュは接続先とユーザーごとに分離し、ログアウト時に削除します。会話の閲覧権限を失ったことを確認した場合も、その会話のキャッシュを削除します。
公開アイコン・スタンプ画像は別のキャッシュにアカウント別で保存し、ログアウト後も保持します。全体64MB・512件の上限を超えると、最後に参照した時刻が古い画像から削除します。
ユーザー情報は `avatar_url` を含む Electric の users shape を取得します。

通話中にアプリを強制終了した場合は、次回起動時の通話一覧更新または新しい通話の開始時に、
この端末の前回の通話セッションへ退出を送ります。入室前に接続先・アカウント別の記録を
Keychainへ保存し、退出成功後に削除します。通信に失敗した場合は記録を保持して再試行します。
退出に失敗した記録があっても、ほかの記録の回収を続けます。新規発信・着信応答では同じ
チャンネルの回収を優先します。通話開始と一覧更新は最大2秒だけ退出完了を待ち、
期限後も回収を続けながら処理を進めます。退出結果が一覧に反映されるのは次回以降の更新です。
回収が長引いて満員の状態が残る場合は再参加が必要です。
認証切れの検出や新しい記録の保存失敗では通話を中止します。
読めない形式の記録は対象アカウントだけ
初期化し、回収できなかった古い参加状態はサーバー側の切断検出に従います。
他の参加者や別端末の通話は終了しません。強制終了から再起動までの間はサーバー側の切断検出に従い、
音声通話の自動再開は行いません。

2026-11-21にビルド、iOS 26.5のSwift Testing 360件・XCTest 12件が成功しました。
初回変更では通話関連UIテスト7件も成功しています。
回復テスト20関数（引数の展開後は29ケース）では再起動、通信失敗と再試行、記録の保存失敗、ログアウト中断、
アカウント分離、遅い退出応答を確認し、reviewer-jpのレビューを通過しました。
実機での強制終了後の本番参加状態は未検証です。

## 外部ライブラリ

音声通話には [stasel/WebRTC](https://github.com/stasel/WebRTC/tree/157.0.0) の配布バイナリを使用し、バージョンを固定しています。
[ライセンス原文](Sources/Resources/WebRTC-LICENSE.txt) をアプリにも同梱します。

## TestFlightへの配布と内部テスター招待

2026-11-21にXcode Cloudの「mainから内部テスト配信」を設定しました。
mainの更新を契機にZooスキームをアーカイブし、内部グループ「zli」へ配信します。
[導入PR #83の記録](https://github.com/Zli-UoA/zoo2-ios/pull/83)では、`02143f7`からのビルド21が成功し、
TestFlight 1.0（21）が「テスト中」になったことを確認しています。
ワークフローの開始条件と配布先はApp Store Connectで管理し、リポジトリには製品の接続情報を保存しています。

GitHub Actionsから、App Store Connectへのユーザー招待と、承諾済みユーザーの内部グループ「zli」への追加を実行できます。
設定方法・実行方法は[内部テスター管理](Scripts/TestFlight/README.md)を参照してください。

## 検証

以下は各変更時の検証履歴です。文書の更新時にアプリの全テストや実機検証を再実行した記録ではありません。
配布の成功と、Siri・Watch・通知・音声通話が実機で動作することは分けて確認します。

2026-11-21 に、発信側のCallKit登録と1人待機中の音声維持を修正し、関連単体テスト74件がiOS 27.0シミュレーターで成功しました。実WebRTC接続・実AVAudioEngineの継続稼働と停止・発信の中断と失敗・音声有効化前の退出・再接続時のミュート・Siri発信を含みます。実機で見つかった待機入力コールバックのMainActor隔離違反も、修正前に同じクラッシュが起きる回帰テストを追加し、`@Sendable`の明示後に成功することを確認しました。着信の参加・拒否、模擬通話のバックグラウンド復帰、通話招待のUIテスト4件と、実機向けビルドも成功しました。reviewer-jpの指摘4件を修正し、クラッシュ修正後の再レビューも通過しています。ロック中の実機での長時間待機と標準通話画面の表示は、引き続き実機確認が必要です。

2026-11-20 に、SwiftData同期・添付キャッシュ・履歴の自動読み込みとIssue #37〜#43の実装後、
Swift Testing 256件・XCTest 12件の計268件が成功しました。
iOS 27.0のiPhoneで主要操作のUIテスト10件、iPad（第10世代）でUIテスト2件も成功しました。
画像ギャラリーのテスト操作を修正した後の再実行を含み、チャンネル編集・アーカイブ、
ショートカット、会話の再表示、履歴追加、添付・PDF、キーボード、iPadの画面構成と通話表示を確認しています。
署名付きReleaseアーカイブの作成とreviewer-jpの最終レビューも通過しました。
TestFlightへ1.0（7）をアップロードし、内部グループ「zli」での「テスト中」を確認しました。
WebRTC配布バイナリのdSYMが含まれない警告があり、
WebRTC内部でのクラッシュ解析には制約が残りますが、アップロードは成功しています。

2026-11-19 の結果:

| 対象 | 結果 |
| --- | --- |
| 入力中表示 | iOS 26.5で追加の単体・APIテスト10件を含むSwift Testing 206件・XCTest 12件と関連UI4件が成功。入力者の切り替え・通信失敗・入力停止・遅い応答、キーボード表示中の入力者の出入り、アプリ復帰、返信・下書き保持を確認。reviewer-jpのレビュー通過。本番端末間の通知は未検証 |
| 標準着信・アプリ内の参加と拒否 | iOS 26.5で着信関連の単体テスト11件、参加・拒否の新規UIテスト2件、既存の通話招待・バックグラウンド継続UIテスト2件が成功。実機向けReleaseビルドとreviewer-jpの最終レビュー通過。Apple経由の着信・実音声はサーバー配備後の実機確認が必要 |
| 返信スレッドのUI | iOS 26.5で関連UIテスト6件が成功。親投稿と返信の区切り、参加者・最終返信日時、文字拡大、送信・下書き保持、履歴の移動、キーボード・複数行入力・新着による表示領域の変化を確認。reviewer-jpの最終レビュー通過 |
| main統合時の確認 | iOS 27.0でSwift Testing 145件・XCTest 11件と関連UI12件を確認。入力中の添付メニューが消える問題を修正し、関連UI4件を再検証して成功。reviewer-jpの最終レビューも通過 |
| Swift Testing・XCTest | iOS 26.5で最終117件＋11件が成功。iOS 27.0ではピン操作追加前の113件＋11件が成功 |
| UIテスト20件 | iOS 26.5で全件成功 |
| 入力欄・シートの最終調整後 | 送信・返信、メンション候補、反応者シートの3件を再実行して成功 |
| 入力ツールバーの追加テスト | ダーク表示でメンション・スタンプの挿入と送信に成功 |
| ピン留めの追加テスト | 一覧の長押しと詳細欄からの追加・解除、別端末との競合、保存失敗からの再試行を検証 |
| 画像・動画 | 拡大・再生のUIテストと保存画像の目視確認に成功 |
| 動画の画面内再生・開始時の表示 | iOS 27.0で関連単体7件・UI4件が成功。先頭フレーム、センシティブ保護、再試行、全画面への切替・復帰、スレッドとの往復を確認。PiPの状態保持・復帰処理は単体テスト済み。シミュレーターがPiP非対応のため、小窓での実再生は実機確認が必要。reviewer-jpの再レビュー通過 |
| 画像の全画面表示・拡大操作 | 単体テスト3件（5パターン）と画像・PDF・動画のUIテスト4件がiOS 27.0で成功。縦長画像の全体表示、ピンチ・ダブルタップ拡大、不要な横移動の抑止、下スワイプ・ボタンで閉じる操作を確認 |
| 添付投稿 | 追加の単体テスト8件（16パターン）とUIテスト2件がiOS 26.5で成功。本文なし投稿・返信・途中失敗後の再送・ファイルの解放・選択キャンセル・添付一覧と削除を確認 |
| PDF表示 | 単体テスト6件（17パターン）とUIテスト2件がiOS 27.0で成功。会話内の1ページ目プレビュー、タップ後の全画面表示・拡大・ページ移動、破損PDFの再読み込みとセンシティブ保護を確認 |
| バックグラウンド通話の回帰確認 | 模擬通話でホーム画面へ移動・復帰後のミュート維持と明示終了、新旧Keychain項目の移行・更新を確認 |
| チャンネル通話の招待 | iOS 26.5で関連単体・APIテスト18件とUIテスト3件が成功。選択・再送・取り消し・同期の遅延、DMでの非表示、バックグラウンド通話を確認。reviewer-jpの指摘2件を修正して再レビュー通過 |
| Releaseの実機向けビルド | 最終実装を署名なしでコンパイル成功 |
| 独立した差分レビュー | reviewer-jpの指摘を修正し、再レビュー通過 |

通話はSSEの受信、API形式、入室前の退出、権限拒否、ミュート、切断と終了を自動テストしています。
実機間の音声品質・Bluetooth・TURN経由の接続、画面ロック中の長時間通話は未検証です。バックグラウンドのUIテストは模擬メディアを使用し、実音声・SSE・heartbeatの継続を保証するものではありません。単独で参加者を待つ状態も実機確認が必要です。

`make test` は本番へのログインや投稿を行いません。ネットワーク境界はテスト専用の
`URLProtocol`、UI 操作は Debug 限定のインメモリ実装を使用します。
`--uitesting-workspace` でネイティブのチャット画面、`--uitesting-send-failure` で送信失敗を再現できます。
これらの起動引数は Release では無効です。
`--uitesting-attachments` はローカルのテストファイルを添付下書きへ追加し、削除・再送の画面操作を検証します。実機のiCloud Driveからの選択と、本番ストレージへのアップロードは未検証です。

認証の承認待ち・キャンセル・期限切れ、トークン更新、通知登録解除に加え、
Electric の初期取得・部分更新・削除・shape 交換、送信失敗後の再送、
送信中の下書き変更、ログアウト後の古い応答、通知のアカウント確認を検証します。
UI テストでは日本語の送信、返信、編集、失敗後の再送、暗号化チャンネルの送信禁止、
長い履歴の初回表示、チャンネルの作成と参加、ログアウトを確認します。
タブ間の下書き保持、参加中のスレッドや通知から会話を開く操作、公開プロフィールの編集も検証します。
編集画面は Command-S でも保存できます。

2026-11-18 に iOS 26.5 シミュレーターから本番の保存済みセッションを復元し、
承認されたチャンネルの履歴取得、テスト投稿 1 件、同じ投稿の編集と再表示を確認しました。
新規ログインは 2026-11-12 に確認済みです。2026-11-19 に開発署名の iPhone 16 Pro Max で、本番バックエンドへの通知登録と APNs サンドボックス経由のテスト通知受信を確認しました。
画面の記録は `artifacts/native-ios-2026-11-18/` にテスト用データだけを保存しています。
詳細は[構成と通信仕様](docs/architecture.md)と[認証・通知の配備](docs/native-auth-push.md)を参照してください。

## 移行の履歴

ネイティブの認証・通知処理は、`Zli-UoA/zoo2` の
`96b1538e134cf211638ffde9a8d6a24cb5be7e16` から移しました。
2026-11-12 には既存のデバイスコード認証で本番のトークン・ユーザー情報取得を確認済みでしたが、
Web 連携の完了待ちでチャットを開けませんでした。
2026-11-18 にチャット画面を SwiftUI に置き換え、WebPage と JavaScript ブリッジへの依存を取り除きました。
この変更ではサーバーのデプロイ、DB のマイグレーション、GitHub への push は行っていません。
