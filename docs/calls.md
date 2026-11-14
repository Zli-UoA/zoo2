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
| Web のタブ | Electric の Shape `GET /api/shapes/call_invites`（呼ばれた人と呼んだ人の `pending` だけ。where はサーバーで固定）。ダイアログと呼び出し音を出す（Web の PR で追加） |
| Web Push | `notificationType: "call_invite"`。`TTL: 30`、`Urgency: high`。`tag` は `zoo-call-{招待の ID}` |
| デスクトップ版 | Shape を購読してトーストを出す（デスクトップ版の PR で追加） |

- 招待が終わったら（応答・拒否・取り消し・不在）、同じ `tag` の `call_invite_end` を Web Push で送り、端末に出ている着信の通知を閉じさせる。
- 応答不可（DND）のユーザーには、着信の Web Push も、その取り消し（`call_invite_end`）も送らない。不在着信の通知（`notification_events` の行）は DND でも残る（Web Push の送信だけ、他の通知と同じく DND に従う）。
- 着信の Web Push は、購読を順に送るので、各送信の直前に、招待の残り時間（送り始めから 30 秒まで）を TTL にし、残りがなければ送らない。
- `call_invite_end` の TTL は 60 秒。Service Worker は、同じ `tag` の通知を閉じる。
- Shape の where: `(invitee_id = $me OR inviter_id = $me) AND status = 'pending'` に加え、いま見られるチャンネルに限る。クライアントの where は使わない。

## クライアントごとの範囲

- Web（React）: 着信ダイアログ・呼び出し音・招待の UI・Web Push の応答/拒否ボタン。バックエンドの PR で、通知一覧の「不在着信」のカードと、Service Worker の `call_invite_end`（出ている通知を閉じる）を先に入れてある。
- デスクトップ版（Tauri）: トースト。Linux は応答/拒否のボタン付き。GPUI 版（`desktop/`）は、`Toast.message_id` が `Option` になったことに合わせて直しただけで、通話の着信は出さない。
- TUI: 通話の機能がないので、呼び出しも扱わない。`message_id` が NULL の行（不在着信）を読み飛ばさず扱えるよう、`client-types` の型だけ合わせた。
- Compose（Android）: 通話の UI は追加しない（通話関連の変更は docs と PR 本文に記録して済ませる方針）。例外として、不在着信（`missed_call`、`message_id` が NULL）は、通知の種類を知らない一覧が「返信」と誤って表示するので、Android の通知一覧と未読数から除く（`frontend-compose/` の `androidNotificationRows`）。
