# Android 作り直しブランチの実キャプチャ

このキャプチャは、新しい Compose 部品を API 35 の Robolectric を日本語ロケールに設定し、架空データとともに描画したもの。360×760dpを2倍密度（720×1520px）のPNGで保存している。端末のステータスバーやキーボード、ブラウザ、通話・写真への実接続は検証対象外。

| 画面 | ライト | ダーク |
| --- | --- | --- |
| ホーム | [ライト](images/android-design/rebuild/home-light.png) | [ダーク](images/android-design/rebuild/home-dark.png) |
| チャンネル一覧 | [ライト](images/android-design/rebuild/channels-light.png) | [ダーク](images/android-design/rebuild/channels-dark.png) |
| 通知 | [ライト](images/android-design/rebuild/notifications-light.png) | [ダーク](images/android-design/rebuild/notifications-dark.png) |
| Bubble | [ライト](images/android-design/rebuild/bubble-light.png) | [ダーク](images/android-design/rebuild/bubble-dark.png) |
| 設定 | [ライト](images/android-design/rebuild/settings-light.png) | [ダーク](images/android-design/rebuild/settings-dark.png) |
| 会話・待機中 | [ライト](images/android-design/rebuild/conversation-idle-light.png) | [ダーク](images/android-design/rebuild/conversation-idle-dark.png) |
| 会話・入力中 | [ライト](images/android-design/rebuild/conversation-light.png) | [ダーク](images/android-design/rebuild/conversation-dark.png) |
| 全画面入力 | [ライト](images/android-design/rebuild/composer-light.png) | [ダーク](images/android-design/rebuild/composer-dark.png) |
| スレッド | [ライト](images/android-design/rebuild/thread-light.png) | [ダーク](images/android-design/rebuild/thread-dark.png) |
| メッセージの長押しメニュー | [ライト](images/android-design/rebuild/message-menu-light.png) | [ダーク](images/android-design/rebuild/message-menu-dark.png) |
| ホーム投稿の長押しメニュー | [ライト](images/android-design/rebuild/post-menu-light.png) | [ダーク](images/android-design/rebuild/post-menu-dark.png) |
| 投稿詳細 | [ライト](images/android-design/rebuild/post-light.png) | [ダーク](images/android-design/rebuild/post-dark.png) |
| 新しい投稿・ActivityPub配信オン | [ライト](images/android-design/rebuild/post-compose-light.png) | [ダーク](images/android-design/rebuild/post-compose-dark.png) |
| 新しい投稿・ActivityPub配信オフ | [ライト](images/android-design/rebuild/post-compose-federation-off-light.png) | [ダーク](images/android-design/rebuild/post-compose-federation-off-dark.png) |
| プロフィール | [ライト](images/android-design/rebuild/profile-light.png) | [ダーク](images/android-design/rebuild/profile-dark.png) |
| チャンネル検索 | [ライト](images/android-design/rebuild/directory-light.png) | [ダーク](images/android-design/rebuild/directory-dark.png) |
| チャンネル作成 | [ライト](images/android-design/rebuild/channel-edit-light.png) | [ダーク](images/android-design/rebuild/channel-edit-dark.png) |
| チャンネル設定 | [ライト](images/android-design/rebuild/channel-settings-light.png) | [ダーク](images/android-design/rebuild/channel-settings-dark.png) |
| チャンネルのメンバー | [ライト](images/android-design/rebuild/channel-members-light.png) | [ダーク](images/android-design/rebuild/channel-members-dark.png) |
| チャンネルのメンバー追加 | [ライト](images/android-design/rebuild/channel-add-member-light.png) | [ダーク](images/android-design/rebuild/channel-add-member-dark.png) |
| 設定の子画面 | [ライト](images/android-design/rebuild/settings-detail-light.png) | [ダーク](images/android-design/rebuild/settings-detail-dark.png) |
| ログイン | [ライト](images/android-design/rebuild/login-light.png) | [ダーク](images/android-design/rebuild/login-dark.png) |
| ログイン復元中 | [ライト](images/android-design/rebuild/login-restoring-light.png) | [ダーク](images/android-design/rebuild/login-restoring-dark.png) |
| 着信（相手のアバターと一定間隔の波紋） | [ライト](images/android-design/rebuild/call-light.png) | [ダーク](images/android-design/rebuild/call-dark.png) |
| 発信中・最小化操作 | [ライト](images/android-design/rebuild/call-outgoing-light.png) | [ダーク](images/android-design/rebuild/call-outgoing-dark.png) |
| 通話中・最小化とPiPの操作 | [ライト](images/android-design/rebuild/call-active-light.png) | [ダーク](images/android-design/rebuild/call-active-dark.png) |
| スレッド設定 | [ライト](images/android-design/rebuild/thread-settings-light.png) | [ダーク](images/android-design/rebuild/thread-settings-dark.png) |
| スレッド参加者 | [ライト](images/android-design/rebuild/thread-members-light.png) | [ダーク](images/android-design/rebuild/thread-members-dark.png) |
| グループ DM 作成 | [ライト](images/android-design/rebuild/group-dm-light.png) | [ダーク](images/android-design/rebuild/group-dm-dark.png) |
| メッセージ・投稿の編集 | [ライト](images/android-design/rebuild/entry-edit-light.png) | [ダーク](images/android-design/rebuild/entry-edit-dark.png) |
| ユーザー検索 | [ライト](images/android-design/rebuild/people-search-light.png) | [ダーク](images/android-design/rebuild/people-search-dark.png) |
| Bubble 参加者 | [ライト](images/android-design/rebuild/bubble-participants-light.png) | [ダーク](images/android-design/rebuild/bubble-participants-dark.png) |

```sh
cd frontend-compose
./gradlew -Pzoo.android=true -Pzoo.uiPreview=true :ui-preview:testDebugUnitTest
```

着信キャプチャは、アプリ内とロック画面が共有するCompose画面のサンプル。端末が実際に表示するロック画面、PiPウィンドウ、SIP回線設定はこの一覧に含まれない。

暗号化の初期設定と解錠は、接続した環境で検証する。リカバリーキーが出る画面では端末のスクリーンキャプチャを無効にする。

生成先は `frontend-compose/ui-preview/build/reports/android-ui/`。
