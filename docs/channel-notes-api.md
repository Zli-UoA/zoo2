# チャンネルノート API

チャンネル内で共有するノートを管理する。すべてのレスポンスに `Cache-Control: no-store` を付ける。

| メソッド | パス | 内容 |
| --- | --- | --- |
| GET | `/api/channels/{channel_id}/notes` | 有効なノートを表示順で 1 ページ分取得（`limit`・`after`） |
| POST | `/api/channels/{channel_id}/notes` | `title`、`content` を指定して作成 |
| GET | `/api/channels/{channel_id}/notes/{note_id}` | ノートを取得 |
| PATCH | `/api/channels/{channel_id}/notes/{note_id}` | `title`、`content`、`version` を指定して更新 |
| DELETE | `/api/channels/{channel_id}/notes/{note_id}` | JSON の `version` を指定して削除 |
| GET | `/api/channels/{channel_id}/notes/deleted` | 24 時間以内に削除されたノートを 1 ページ分取得（`limit`・`after`） |
| POST | `/api/channels/{channel_id}/notes/{note_id}/restore` | 削除後 24 時間以内のノートを復元 |
| PATCH | `/api/channels/{channel_id}/notes/order` | `expected_note_ids` と新しい `note_ids` を指定して並べ替え |

公開チャンネルの閲覧は通常のユーザーに許可し、ゲストと非公開チャンネル・DM は参加者に限定する。作成・変更・削除・復元・並べ替えと削除済み一覧の閲覧は参加者のみ可能。アーカイブ済みチャンネルは閲覧のみ可能。

削除後 24 時間を過ぎたノートは、定期処理により DB から物理削除する。停止中や障害時は遅れる場合があり、復旧後に再試行する。期限後は物理削除前でも復元できない。

API のノート作成・更新・削除・復元・並べ替えは、順序や対象行を読む前にチャンネル単位の transaction advisory lock (`pg_advisory_xact_lock(hashtextextended('channel-notes:' || channel_id, 0))`) を取得して直列化する。本文や削除状態を直接 SQL で変更する場合は、同じトランザクションで対象ノートの `version` も進めること。ロックだけでは、変更前に同じ版を取得したクライアントの更新を拒否できない。異なるチャンネルの API 書き込みは独立して進む。直接 SQL でノートを書き込む場合は、同じトランザクションで、最初の関連読み取りより前（読み取りをしない場合は書き込み前）に対象チャンネルの同じ lock を取得する。さらに、平文ガードの trigger (`e2ee_reject_plaintext_channel_notes`) とノート API の認可が最初に取る `channels` の `FOR SHARE` と同じロック順序に従い、`channel_notes` への直接 UPDATE・DELETE の前に同じチャンネル行を `SELECT ... FOR SHARE` で共有ロックしてから書くこと。trigger は `channel_notes` の行を更新する前に `channels` を `FOR SHARE` するため、この順序を外れると trigger・API・直接 SQL の間でデッドロックし得る。複数チャンネルのノートを移動・更新する SQL は、デッドロックを避けるため channel ID の昇順で各 lock を取ってから読み書きする。これは協調する書き込み経路間の契約であり、DB が任意 SQL のロック取得や認可・version 検証・順序の意味を透過的に保証するものではない。運用上の直接 SQL や保守ジョブもこの契約に従うこと。チャンネル削除の FK cascade はチャンネル行の排他ロックが API の共有ロックと競合するため、API 書き込み完了後に実行される。E2EE の DB guard は別の trigger で維持する。DB は active note の `position` 一意性も制約していないため、任意の値を指定する SQL や trigger を無効化する管理操作は重複位置を作り得る。

ノートは平文で保存されるため、暗号化チャンネルでは利用できない。有効なノートが残るチャンネルと、削除済みでも復元できる 24 時間の間は暗号化への切替を拒否する。タイトルは空白を除いて 1〜200 文字、本文は最大 100,000 文字。有効なノートは 1 チャンネルあたり 500 件までとし、上限に達している状態で作成または復元すると `409 Conflict` を返す。一覧・並べ替え・復元の処理量は最大 500 件を上限とする。直接 SQL でノートを書き込む場合も、同じ上限を適用する。ノートロック契約に従う直接 SQL は任意の `position` を書けるが、有効ノートの最大 `position` が `i32::MAX` に達した状態での作成、および復元時の位置ずらし (`position + 1`) が `i32::MAX` の行に当たる場合は、API は有効ノート全体を `(position, id)` 順の連番 `0, 1, 2, ...` に詰め直してから挿入・ずらしを行う。詰め直しは相対順序を変えないため、既存の表示順は保たれる。

項目 API は一覧と同じ形式の `note_id` をそのまま受け付ける。作成 API は UUID を保存するが、ノートロック契約に従う直接 SQL は任意の text id を書き得るため、取得・更新・削除・復元も保存済みの id を拒否しない。チャンネル ID は UUID である。

更新と削除には取得時の `version` が必要で、対象がまだ有効で `version` が一致しない場合は `409 Conflict` を返す。別の参加者が対象を削除済みの場合（削除前の `version` を送ったときを含む）は `404 Not Found` を返す。この 2 つを区別することで、クライアントは競合の再試行と削除の反映済みを切り分けられる。並べ替えも現在の順序が `expected_note_ids` と一致しない場合は `409 Conflict` を返す。現在順序は一覧と同じ SQL の `ORDER BY position, id`（DB の照合順序）で求めるため、クライアントは一覧レスポンスの並びをそのまま `expected_note_ids` に使える。

有効ノートの一覧は本文を返さない (メタデータのみ: `id, channel_id, title, position, version, created_user_id, created_at, updated_at, order_token`)。本文は `GET /api/channels/{channel_id}/notes/{note_id}` で別途取得する。一覧は `(position, id)` の昇順で 1 ページの件数を SQL で制限する。クエリーパラメーター `limit`（1〜200、既定 50）と `after`（前ページ最後の項目の `order_token`・`position`・`id` を空白区切りにした値）で続きのページを取得でき、全ノートに同じ表示順でアクセスできる。

各項目の `order_token` はチャンネルの有効ノート全体の `(position, id)` に対するハッシュ (`md5`) で、同一チャンネル・同一ページ内では全項目が同じ値を持つ。ノートの追加・削除・復元・並べ替えのいずれでも変化するため、ページ継続の際にサーバーが前ページ取得時の並び・構成と同じであることを検証する素材になる。`after` のトークンが現在のものと一致すれば `(position, id) > (<position>, <id>)` で続きを返し、一致しなければ（ページングの途中で並びや構成が変われば）`409 Conflict` を返す。この場合、位置カーソルだけでは新しい並びの続きを安全に特定できないため、最初のページから再読み込みすること。期限・メンバーシップ・可視性の条件は全ページで維持される。

`after` の `id` 部分（3 番目の要素）は、区切りの空白が存在する限り空文字 (`""`) を許す。ノートロック契約に従う直接 SQL は任意の text id を書けるため、空の id がページ境界に来ても `"<order_token> <position> "` のカーソルで継続できる。区切りそのものの欠落、空のトークン・position、数値でない position は `400 Bad Request` になる。

削除済み一覧は `(deleted_at, id)` の昇順で、1 ページの件数を SQL で制限する。クエリーパラメーター `limit`（1〜200、既定 50）と `after`（前ページ最後の項目の `deleted_at` と `id` を空白区切りにした値）で次ページを取得できる。各項目の `deleted_at` は UTC の `YYYY-MM-DDTHH:MM:SS.mmm`（23 文字・ミリ秒 3 桁固定）で返し、カーソルも同じ形式に限る。この形式に合わない `deleted_at`（ミリ秒欠落・実在しない日時など）は SQL の実行前に `400 Bad Request` として拒否する（キャスト失敗の 500 にはしない）。`after` の `id` 部分は有効一覧と同じく、区切りの空白があれば空文字を許す。期限（24 時間）とチャンネルの条件は全ページで維持される。

メンション通知とクライアント画面は別の変更で追加する。
