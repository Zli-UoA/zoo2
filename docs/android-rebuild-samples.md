# Android 作り直しブランチの実キャプチャ

このキャプチャは、新しい Compose 部品を API 35 の Robolectric で架空データとともに描画したもの。端末のステータスバーやキーボード、ブラウザ、通話・写真への実接続は検証対象外。

| 画面 | ライト | ダーク |
| --- | --- | --- |
| ホーム | [ライト](images/android-design/rebuild/home-light.png) | [ダーク](images/android-design/rebuild/home-dark.png) |
| チャンネル一覧 | [ライト](images/android-design/rebuild/channels-light.png) | [ダーク](images/android-design/rebuild/channels-dark.png) |
| 通知 | [ライト](images/android-design/rebuild/notifications-light.png) | [ダーク](images/android-design/rebuild/notifications-dark.png) |
| Bubble | [ライト](images/android-design/rebuild/bubble-light.png) | [ダーク](images/android-design/rebuild/bubble-dark.png) |
| 設定 | [ライト](images/android-design/rebuild/settings-light.png) | [ダーク](images/android-design/rebuild/settings-dark.png) |
| 会話 | [ライト](images/android-design/rebuild/conversation-light.png) | [ダーク](images/android-design/rebuild/conversation-dark.png) |
| 全画面入力 | [ライト](images/android-design/rebuild/composer-light.png) | [ダーク](images/android-design/rebuild/composer-dark.png) |
| スレッド | [ライト](images/android-design/rebuild/thread-light.png) | [ダーク](images/android-design/rebuild/thread-dark.png) |
| 投稿詳細 | [ライト](images/android-design/rebuild/post-light.png) | [ダーク](images/android-design/rebuild/post-dark.png) |
| 新しい投稿 | [ライト](images/android-design/rebuild/post-compose-light.png) | [ダーク](images/android-design/rebuild/post-compose-dark.png) |
| プロフィール | [ライト](images/android-design/rebuild/profile-light.png) | [ダーク](images/android-design/rebuild/profile-dark.png) |
| チャンネル検索 | [ライト](images/android-design/rebuild/directory-light.png) | [ダーク](images/android-design/rebuild/directory-dark.png) |
| チャンネル作成 | [ライト](images/android-design/rebuild/channel-edit-light.png) | [ダーク](images/android-design/rebuild/channel-edit-dark.png) |
| チャンネル設定 | [ライト](images/android-design/rebuild/channel-settings-light.png) | [ダーク](images/android-design/rebuild/channel-settings-dark.png) |
| 設定の子画面 | [ライト](images/android-design/rebuild/settings-detail-light.png) | [ダーク](images/android-design/rebuild/settings-detail-dark.png) |
| ログイン | [ライト](images/android-design/rebuild/login-light.png) | [ダーク](images/android-design/rebuild/login-dark.png) |
| 着信 | [ライト](images/android-design/rebuild/call-light.png) | [ダーク](images/android-design/rebuild/call-dark.png) |

```sh
cd frontend-compose
./gradlew -Pzoo.android=true -Pzoo.uiPreview=true :ui-preview:testDebugUnitTest
```

生成先は `frontend-compose/ui-preview/build/reports/android-ui/`。
