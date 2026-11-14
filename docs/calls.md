# 音声通話と呼び出し

チャンネルの通話は、ブラウザー同士の P2P（WebRTC のメッシュ、最大 6 人）です。シグナリングは SSE（`GET /api/channels/{id}/call/stream`）で、参加者は `call_presence` に置きます。このページは、通話の中で相手を呼び出す「招待」と、呼ばれた側の着信通知をまとめます。実装は `backend/src/routes/calls.rs` と `backend/src/call_invites.rs` です。

## 呼び出しの流れ

- **DM**: 空の部屋に最初の人が入ると（自分以外に誰もいなければ。切断に気づく前の自分の古いセッションは数えない）、DM の相手（有効な人間のユーザー）を自動で呼ぶ。
- **チャンネル**: 通話に入っている人が、`POST /api/channels/{id}/call/invites`（`{ "user_ids": [...] }`）でメンバーを選んで呼ぶ。DM では使えない（400）。
  - 呼べるのは、そのチャンネルを見られる人だけ（非公開チャンネルと guest はメンバーのみ）。見られない人と存在しない人は、区別のつかない 404 にする。
  - すでに通話にいる人と、呼び出し中の人は、新しく呼ばない（`invites` に入らない）。通話の定員（`MAX_CALL_PARTICIPANTS`）を超える呼び出しはしない。空きの数（定員 - 在室 - 呼び出し中）までしか作らず、1 回の要求で多く指定しても、空きを超えた分は作られない。同時の要求や入室と競合しても超えないよう、空きの確認から作成までを、通話への参加と同じチャンネル行のロックの下で行う。グループ DM の自動呼び出しも、同じ空きまでしか呼ばない。通話への参加の判定も、呼び出し中の人と、応答してから 60 秒以内でまだ入室していない人（どちらも自分以外）の席を空けておく（在室 + 予約 < 定員）。Web は応答の API のあとで入室するので、マイクの許可を待つ間も席を失わない。入室済みの人は、在室に数えるので、予約には数えない。入室したら `call_invites.joined_at` に時刻を残して、予約を使い終えたことにする（そのあと 60 秒以内に退出しても、予約は戻らない）。応答済みで未入室の人は、保持の間、呼び直さない。招待されていない人が先に入って、呼ばれた人が「満室」で入れなくなることを防ぐため。
- 呼ばれた人は応答するか拒否する。呼んだ人は取り消せる。
  - `POST /api/call-invites/{id}/accept`、`/decline`: 呼ばれた本人だけ。
  - `POST /api/call-invites/{id}/cancel`: 呼んだ本人だけ。
  - 他人の招待は存在を見せず 404。すでに終わった招待は 409。期限を過ぎた招待への応答も 409。
  - `accept` は招待を `accepted` にして `channel_id` を返す。通話への参加は、続けて `GET /call/stream` で行う。

## 招待の状態

`call_invites.status` は次のとおり。

| 状態 | きっかけ |
| --- | --- |
| `pending` | 呼び出し中。作成から 30 秒（`expires_at`）が期限 |
| `accepted` | 呼ばれた人が応答した。または、どれかの端末で通話に入った |
| `declined` | 呼ばれた人が拒否した |
| `cancelled` | 呼んだ人が取り消した。または、部屋が空になった（呼んだ人が全員抜けた） |
| `missed` | 期限までに応答がなかった |

同じ人を同じチャンネルで同時に呼べるのは 1 件だけ（`(channel_id, invitee_id) WHERE status = 'pending'` の部分一意インデックス）。

定期処理（`call_invites::sweep`。3 秒ごと。複数のレプリカが走っても `FOR UPDATE SKIP LOCKED` で二重に処理しない）が、期限を過ぎた `pending` を `missed` にして、同じ文で不在着信の通知を作る（通知を作ったものにだけ push も送る）。呼び出しと入室が同時に起きて、入室済みの人に `pending` が残ったときも、ここで `accepted` にする。空の部屋の `pending` の取り消しも、ここで拾う（通話を抜けたとき、`leave` や SSE の切断でも即時に行う）。

## 不在着信

`notification_events` に `source = 'channel'`、`notification_type = 'missed_call'` の行を作り、未読にする。

- `actor_id` は呼んだ人、`channel_id` は通話のチャンネル。**`message_id` は NULL**（メッセージがないため）。CHECK 制約は、`missed_call` のときだけ `message_id` を省略できる。`message_deleted` は `false`（Electric の通知 Shape から消えないように、`set_notification_message_deleted` が `message_id` なしの行を `false` にする）。
- 通知の ID は招待の ID から決まる値（`md5('missed_call:' || id)` の UUID 形式）なので、定期処理をやり直しても重複しない。
- 旧 `notifications` テーブル（ローリングデプロイ用の互換）には書かない。`message_id` が NULL の行は、`sync_notification_events` が旧テーブルへ写さない。
- 通知の API の `message_id` は `null` になりうる。既読化・一覧・コンテキスト取得は、メッセージなしで動く。
- 古いクライアント（`message_id` を必須とする TUI・デスクトップ版の旧版）は、この行を不正な行として読み飛ばす（`Store::invalid_rows`）。

## 届け先

| 届け先 | 方法 |
| --- | --- |
| Web のタブ | Electric の Shape `GET /api/shapes/call_invites`（呼ばれた人と呼んだ人の `pending` だけ。where はサーバーで固定）。ダイアログと呼び出し音を出す |
| Web Push | `notificationType: "call_invite"`。`TTL: 30`、`Urgency: high`。`tag` は `zoo-call-{招待の ID}` |
| デスクトップ版 | Rust 側が Shape を購読してトーストを出す（下の「デスクトップ版の動き」） |

- 招待が終わったら（応答・拒否・取り消し・不在）、同じ `tag` の `call_invite_end` を Web Push で送り、端末に出ている着信の通知を閉じさせる。
- 応答不可（DND）のユーザーには、着信の Web Push も、その取り消し（`call_invite_end`）も送らない。不在着信の通知（`notification_events` の行）は DND でも残る（Web Push の送信だけ、他の通知と同じく DND に従う）。
- 着信の Web Push は、購読を順に送るので、各送信の直前に、招待の残り時間（送り始めから 30 秒まで）を TTL にし、残りがなければ送らない。
- `call_invite_end` の TTL は 60 秒。Service Worker は、同じ `tag` の通知を閉じる。
- Shape の where: `(invitee_id = $me OR inviter_id = $me) AND status = 'pending'` に加え、いま見られるチャンネルに限る。クライアントの where は使わない。

## クライアントごとの範囲

- Web（React）: 着信ダイアログ・呼び出し音・招待の UI・Web Push の応答/拒否ボタン（下の「Web の動き」）。バックエンドの PR で、通知一覧の「不在着信」のカードと、Service Worker の `call_invite_end`（出ている通知を閉じる）を先に入れてある。
- デスクトップ版（Tauri）: トースト。Linux は応答/拒否のボタン付き（下の「デスクトップ版の動き」）。GPUI 版（`desktop/`）は、`Toast.message_id` が `Option` になったことに合わせて直しただけで、通話の着信は出さない。
- TUI: 通話の機能がないので、呼び出しも扱わない。`message_id` が NULL の行（不在着信）を読み飛ばさず扱えるよう、`client-types` の型だけ合わせた。
- Compose（Android）: 通話の UI は追加しない（通話関連の変更は docs と PR 本文に記録して済ませる方針）。例外として、不在着信（`missed_call`、`message_id` が NULL）は、通知の種類を知らない一覧が「返信」と誤って表示するので、Android の通知一覧と未読数から除く（`frontend-compose/` の `androidNotificationRows`）。

## Web の動き

- **購読**: `callInvites`（`frontend/src/lib/electric/collections.ts`）が `/api/shapes/call_invites` を購読する（永続化しない）。`useCallInvites` が、自分が呼ばれている招待（`incoming`）と、自分が呼んでいる招待（`outgoing`）に分ける。サーバーが `missed` にするまでの数秒も、期限を過ぎた招待は一覧から外す。期限は、端末の時計がずれていても働くよう、行を初めて受け取った時刻 + 招待の長さ（`created_at` と `expires_at` の差）で決める。
- **着信**: `IncomingCallDialog`（`routes/_shell.tsx` の `CallProvider` の下）が、`incoming` の先頭を `IncomingCallCard` で出し、着信音（`call-sounds.ts` の `startRingtone`。効果音の設定に従う。ページを操作する前は、ブラウザーの自動再生の制限で鳴らないことがある）を繰り返す。
  - 応答: `POST /api/call-invites/{id}/accept` のあとに `joinCall(channelId)`（`joinCall` は通話が終わるまで戻らないので待たない。待つと、通話中に届く次の着信を操作できない）。別の通話中なら、`joinCall` が先に抜ける。終わっていた招待（409・404）には入らず、「この着信はすでに終了しました」をトーストで出す。それ以外の失敗は、カードに「応答できませんでした」を出す。
  - 拒否: `POST /api/call-invites/{id}/decline`。
- **招待する側**（`CallBar`）: チャンネルの通話では「メンバーを招待」で、通話に入っておらず呼び出し中でもない、チャンネルのメンバー（bot と無効化したユーザーを除く）を選んで呼ぶ（`CallInvitePicker`。空き = 定員 - 在室 - 呼び出し中 まで）。DM では出さない。呼び出し中の相手は「〇〇さんを呼び出し中…」と出し、取り消せる。
- **Web Push**（`public/sw.js`）:
  - `call_invite` は `requireInteraction` つきで、「応答」「拒否」のボタンを付ける。
  - 「応答」: URL だけでは応答しない（リンクを開かせるだけで通話に入れてしまうため）。Service Worker が Cache（`zoo-call-accept`）に「応答を押した」印（招待の ID と時刻）を残し、画面（`IncomingCallDialog`）が、印を一度だけ読んで消せたときだけ、応答して通話に入る。印は 60 秒で無効。
    - 開いているタブがあれば、移動（再読み込み）せずに前へ出し、`postMessage`（`{ type: "zoo-call-invite", action: "accept", inviteId }`）で依頼する。複数のタブのうち、印を先に消せた 1 つだけが応答する。
    - タブがなければ `/channels/{id}?call_invite={招待の ID}` を開く。画面は、URL の `call_invite` を外してから、印があれば応答する。印がなければ（外部のリンクなどで開かれた場合）何もしない。
  - 「拒否」: Service Worker は API のトークンを持たないので、画面に頼む。開いているタブがあれば `postMessage`（`action: "decline"`）で依頼する。タブがなければ、「拒否を押した」印（Cache）を残してチャンネルを開き、画面が印を一度だけ読んで拒否する（URL だけでは拒否しない）。
  - `call_invite_end` は、同じ `tag` の通知を閉じる（通知がなければ、同じ `tag` で出してすぐ閉じる）。
- **応答不可（DND）**: 着信の Web Push は DND の人に送らないが、開いているタブの着信ダイアログと着信音は、DND の状態を見ずに出す（画面を見ている人に隠さない）。DND でも鳴らしたくない場合は、DND の状態をフロントエンドで読む変更が別に要る。
- **不在着信**: 通知一覧の `NotificationCard` に「〇〇さんからの不在着信」を出し、チャンネルへのリンクを付ける（`message_id` は NULL）。

## デスクトップ版の動き

Tauri 版（`desktop-tauri/`）は、ウィンドウが前面にないときだけ、OS のトーストで着信を知らせる。前面にあるときは、画面（Web）の着信ダイアログに任せる。

- **購読**: 通話の招待の Shape（`ShapeId::CallInvites`）だけは、どちらの画面（Leptos・React）でも、Rust 側が購読する（WebView 側の購読とは別。行は通常 0〜数行）。この購読だけが失敗しても（古い backend など）、画面にはエラーを出さない。
- **出す条件**: 自分あての `pending` で、通知の設定がオン、DND でなく（DND の間は出さず、あとから出すこともしない）、ウィンドウが前面にない（表示されていて、フォーカスがある）とき。Leptos の画面には着信のダイアログがないので、前面でも出し、前面に来ても閉じない。DND を初めて取れたときに、それまでに届いていた招待を判定し直す。同じ招待のトーストは 1 回だけ。初回の同期や `reset` で届いた行は、期限（`expires_at`）が過ぎていれば出さない。同期が済んだあとの差分で届いた行は、端末の時計がずれていても出す。
- **題名**: users の Shape を持たない React の画面では、呼んだ人の名前を、通話の参加者（`GET /api/channels/{id}/call`）から API で引く。引けなければ、ユーザー ID は出さず、「着信」の決まった文言にする。
- **閉じる**: Shape から招待が消えたら（応答・拒否・取り消し・不在）、出ているトーストを閉じる。ウィンドウが前面に来たときと、ログアウトでも閉じる。Shape から消えなかったときの保険として、受け取った時刻 + 招待の長さ + 3 秒でも閉じる。
- **Linux**: 「応答」「拒否」のボタン付きで、閉じるまで出し続ける（`Urgency::Critical`、`Timeout::Never`。GNOME は通常の通知を数秒で一覧にしまうため）。本文のクリックは、ウィンドウを前に出すだけで、応答はしない。
  - 「拒否」: Rust 側の API クライアントで `POST /api/call-invites/{id}/decline`。
  - 「応答」: Rust 側が「応答の印」（招待の ID と時刻。60 秒で無効）を残し、ウィンドウを前に出して、`zoo://call-accept` を送る。このイベントは「印を取りに来て」の合図にすぎない。React（`DesktopBridge`）は、`zoo_take_call_accept` で印を一度だけ取れたときだけ、`IncomingCallDialog` に応答を頼む（API の accept と `joinCall`）。起動の直後にも、残った印を取る。Web Push の「応答」の印（Cache）と同じ考え方で、イベントや URL だけでは応答しない。`zoo://open` は招待を扱わない。
  - Leptos の画面には通話の UI がなく、マイクの許可もないので、「応答」のボタンは出さず、「拒否」だけにする。
- **mac**: トーストを出し、招待が終わったら閉じる。クリックは受け取れない（他の通知と同じ）ので、ボタンはなく、応答は画面（ダイアログ）で行う。
- **古い版との組み合わせ**: 古い Web と新しいアプリ: 合図は無視されるが、ウィンドウが前に出て、ダイアログで応答できる。新しい Web と古いアプリ: `zoo_take_call_accept` がなく、何もしない。`PROTOCOL_VERSION` は上げない。
