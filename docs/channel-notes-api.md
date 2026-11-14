# チャンネルノート API

チャンネル内で共有するノートを管理する。すべてのレスポンスに `Cache-Control: no-store` を付ける。

| メソッド | パス | 内容 |
| --- | --- | --- |
| GET | `/api/channels/{channel_id}/notes` | 有効なノートを表示順で取得 |
| POST | `/api/channels/{channel_id}/notes` | `title`、`content` を指定して作成 |
| GET | `/api/channels/{channel_id}/notes/{note_id}` | ノートを取得 |
| PATCH | `/api/channels/{channel_id}/notes/{note_id}` | `title`、`content`、`version` を指定して更新 |
| DELETE | `/api/channels/{channel_id}/notes/{note_id}` | JSON の `version` を指定して削除 |
| GET | `/api/channels/{channel_id}/notes/deleted` | 24 時間以内に削除されたノートを取得 |
| POST | `/api/channels/{channel_id}/notes/{note_id}/restore` | 削除後 24 時間以内のノートを復元 |
| PATCH | `/api/channels/{channel_id}/notes/order` | `expected_note_ids` と新しい `note_ids` を指定して並べ替え |

公開チャンネルの閲覧は通常のユーザーに許可し、ゲストと非公開チャンネル・DM は参加者に限定する。作成・変更・削除・復元・並べ替えと削除済み一覧の閲覧は参加者のみ可能。アーカイブ済みチャンネルは閲覧のみ可能。

削除後 24 時間を過ぎたノートは、定期処理により DB から物理削除する。停止中や障害時は遅れる場合があり、復旧後に再試行する。期限後は物理削除前でも復元できない。

ノートは平文で保存されるため、暗号化チャンネルでは利用できない。タイトルは空白を除いて 1〜200 文字、本文は最大 100,000 文字。更新と削除には取得時の `version` が必要で、古い版の場合は `409 Conflict` を返す。並べ替えも現在の順序が `expected_note_ids` と一致しない場合は `409 Conflict` を返す。

メンション通知とクライアント画面は別の変更で追加する。
