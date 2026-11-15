# 新規アカウントの参加チャンネル

管理者は「Zoo設定」→「メンバー」の「新規アカウントの参加チャンネル」で、公開チャンネルを複数選択して保存する。すべて外して保存すると自動参加を無効にできる。未設定のときも自動参加しない。

初回 OIDC ログインでアカウントを作成するとき、ユーザー・認証情報・チャンネル参加を同じトランザクションで保存する。開発用の直接ユーザー作成にも適用する。既存アカウントのログインや、退会済みチャンネルへの再参加には適用しない。Android・TUI から初回ログインした場合もサーバーで適用する。既存の認証 API と Electric の形式は変更しない。

対象は有効な公開チャンネルに限る。非公開、DM、アーカイブ済み、削除済みは選べず、保存後にその状態へ変わった場合も参加しない。チャンネル ID を保存するため、名前を変更しても設定は維持する。

## TUI

管理者としてログインした状態で実行する。

```sh
zoo2-tui default-channels                  # 設定と選択可能なチャンネルを表示
zoo2-tui default-channels --set ID1 ID2     # 複数指定
zoo2-tui default-channels --set             # 自動参加を無効化
```

## API

管理者専用の `GET /api/workspace/default-channels` は `channel_ids` と `channels`（`id`・`channel_name`）を返す。`PATCH` に `{"channel_ids":["ID1","ID2"]}` を渡すと置き換える。重複 ID、無効な対象、100 件を超える指定は拒否する。

設定は既存の `workspace_settings.preferences.default_channel_ids` に保存する。DB マイグレーションや環境変数の追加は不要。通常の `PATCH /api/workspace` による preferences 更新では、この予約キーを維持し、専用 API の検証を迂回できないようにする。
