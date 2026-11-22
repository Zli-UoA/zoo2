# 通話の SFU

チャンネルの通話は、少人数のあいだは P2P mesh のまま（[音声通話と呼び出し](calls.md)）です。人数が増えたら、音声を SFU に集めて配ります。SFU は次の 2 つです。

| SFU | 使う人 | つなぎ方 |
| --- | --- | --- |
| LiveKit（zoo2 の k8s） | k8s と同じ LAN にいる人 | シグナリングは Cloudflare Tunnel の `wss`。メディアは LAN のプライベート IP（UDP 7882・TCP 7881） |
| Cloudflare Realtime（SFU API） | それ以外の全員、Android の WebView | Cloudflare の anycast。API は backend が代わりに呼ぶ |

同じ通話に両方の参加者がいるときは、`sfu-bridge`（`sfu-bridge/`、Go）が 2 つの SFU の音声を中継します。kubecamp は HTTP Ingress しかなく、メディアを入れられないので使いません。

## 部屋の mode

- 部屋の状態は `call_rooms(channel_id, mode, epoch, track_version)` に置く。`mode` は `mesh` か `sfu`。
- 在室が 0 人の部屋に入ると、`mesh` に戻して `epoch` を進める。LiveKit のルーム名は `call-{channel_id}-{epoch}`。
- 入室のたびに、チャンネル行のロックの下で、次の 2 つを満たすか調べる。満たせば `sfu` に切り替える。
  - 在室の人数が `CALL_SFU_THRESHOLD`（既定 4）を超えている。
  - 在室の全員が SFU に対応している。
- 切り替えは、入室と同じトランザクションで `mode_changed` を通知する。切り替えたあと、人数が減っても `mesh` には戻さない（空になるまで）。
- SFU に対応しているかは、クライアントが `GET /call/stream?sfu=1` で申告する。対応していないクライアント（iOS、古い Android や Web）がいる間は `mesh` のままで、定員も 6 人のまま。
- `sfu` の部屋に対応していないクライアントが入ろうとしたら、409 `CALL_SFU_UNSUPPORTED` を返す。
- 定員は `capacity(mode)`。`mesh` は 6、`sfu` は `CALL_SFU_MAX_PARTICIPANTS`（既定 50）。呼び出しの空きの計算も同じ定員を使う。
- 入室した人の SSE には、`room_state` の `mode` と `epoch` が入る。自分の入室で切り替わったときは、`mode_changed` が `room_state` より先に届くことがある。

## SFU の割り当て

`POST /api/channels/{id}/call/sfu/join`（`{ session_id, fallback?, cloudflare_only? }`）で、参加者ごとに決める。

1. `CF-Connecting-IP`（`activitypub/client_ip.rs` の信頼の条件で読む）が `lan_egress_cidrs`（LAN の出口のグローバル IP。IPv4・IPv6）に入っていれば LiveKit。
2. それ以外、IP がわからないとき、`fallback`、`cloudflare_only` のときは Cloudflare。
3. bridge が無効のときは、部屋に Cloudflare の人が 1 人でもいれば Cloudflare にする。あとから Cloudflare の人が入ったら、`sfu_reassign` で LiveKit の人に移ってもらう。

- ブラウザーから LAN のプライベート IP を測ること（RTT probe）はしない。公開サイトからプライベート IP へのアクセスは、ブラウザーが制限する（Private Network Access）。
- LiveKit に割り当てられても、8 秒で ICE がつながらなければ、クライアントは `fallback: true` で join し直す。
- LiveKit の JWT（HS256、有効期限 5 分）の権限は次のとおり。`roomAdmin` と `roomCreate` は付けない。
  - 部屋に入れる（`roomJoin`）。部屋は 1 つに固定。
  - publish はマイクだけ（`canPublishSources: [microphone]`）。データは publish できない（`canPublishData: false`）。
  - identity は通話の `session_id`。
- Cloudflare の代理は `cf/publish`、`cf/pull`、`cf/answer` の 3 つだけ。
  - `cf/publish` は、音声の m-line が 1 本だけの offer を受け付ける。
  - `cf/pull` は、backend が配るトラックの id（`{session_id}:{direct|bridge}`）で指定する。backend が同じ部屋の行に引き直すので、クライアントは Cloudflare のセッション ID や track 名を指定できない。
  - app secret は backend だけが持つ。

## トラック一覧

- Cloudflare にある音声は `call_sfu_tracks` に置く。`direct` は話者のクライアントが push したもの、`bridge` は LiveKit の話者を bridge が push したもの。
- 話者の presence が消えると、外部キーで一緒に消える。
- 一覧が変わると `call_rooms.track_version` を進め、`sfu_tracks_changed`（`channel_id`・`epoch`・`version` だけ）を `pg_notify` する。一覧そのものは NOTIFY の上限（8000 バイト）を超えるので載せない。
- 通知を受けた各レプリカが DB から一覧を読み、SSE の `sfu_tracks` で配る。クライアントは、手元より古い `version` を捨てる。

## 退出したときの後始末

- `call_presence` の DELETE のトリガー（`record_call_sfu_departures`）が、SFU に割り当てた人の退出を `call_sfu_departures` に記録する。退出、sweep、キック、チャンネル削除のどの経路でも記録される。
- backend の worker（`CallSfu::start_departure_worker`）は、記録を見て次を呼び、退出した人が音声を送ることも受けることもできないようにする。
  - Cloudflare：`GET /sessions/{id}` と `tracks/close`（force）
  - LiveKit：`RemoveParticipant`
- 失敗したものは 5 回まで試し直す。

## sfu-bridge

- 起動のたびに `POST /internal/sfu-bridge/register` で epoch（ミリ秒の時刻）を受け取る。backend は、それより古い bridge のトラックを消す。
- `GET /internal/sfu-bridge/rooms?bridge_epoch=…&since=…` を long-poll する。返るのは、LiveKit と Cloudflare の両方に人がいる部屋の一覧と、部屋ごとの LiveKit のトークン。変化がなければ最大 25 秒待つ。
- 部屋ごとに LiveKit に `bridge:{epoch}` として入り（hidden にはしない）、Cloudflare のセッションを 1 つ持つ。
  - LiveKit の話者の声：Cloudflare に push する（`/internal/sfu-bridge/cf/publish`、track 名は `{session_id}-bridge`）。
  - Cloudflare の話者の声（`direct`）：pull して、トラック名を話者の `session_id` にして LiveKit に publish する。
- Opus の RTP はトランスコードしない。RTP ヘッダ拡張の ID の割り当ては LiveKit と Cloudflare で違うので、拡張は外す。
- 内部 API の認証は `SFU_BRIDGE_TOKEN`（Bearer）。

## クライアント

| クライアント | SFU |
| --- | --- |
| Web・Tauri | LiveKit と Cloudflare（`frontend/src/lib/call/sfu/`）。`livekit-client` は LiveKit に割り当てられたときだけ読み込む |
| Android | Cloudflare だけ（`frontend/src/android-phone/sfu-call.ts`）。WebView の CSP は外へ接続させない（`connect-src 'none'`）ので、API は Kotlin が代わりに呼ぶ（`sfu_request` → `rtc.sfu.reply`。操作は 4 つに限る） |
| iOS | 未対応。`sfu=1` を付けないので、iOS がいる部屋は mesh のまま |
| TUI | 通話の機能がない |

- mesh の部屋で `mode_changed` を受けたら、SFU につないでから mesh を閉じる。
- `rtc-audio.ts` と `audio-media.ts` は SIP と共有している。SFU の処理は、別のファイルに分けて置いた。`AudioCallMedia` に足したのは、マイクのトラックを読む `micTrack()` だけ。

## 設定

設定は zoo2-infra の PR で入れる。`CALL_SFUS_JSON` が空のあいだは、今までどおり mesh だけで動く。

| 変数 | 必須 | 内容 |
| --- | --- | --- |
| `CALL_SFUS_JSON` | 任意 | `{"cloudflare":{"api_base"?},"livekit"?:{"ws_url","api_url","lan_egress_cidrs":[…]},"bridge"?:bool}`。空なら SFU は無効 |
| `CF_REALTIME_APP_ID`・`CF_REALTIME_APP_SECRET` | `CALL_SFUS_JSON` があれば必須 | Cloudflare Realtime の app（Secret） |
| `LIVEKIT_API_KEY`・`LIVEKIT_API_SECRET` | `livekit` があれば必須 | secret は 32 文字以上（Secret） |
| `SFU_BRIDGE_TOKEN` | `bridge: true` なら必須 | 32 文字以上。backend と sfu-bridge で同じ値（Secret） |
| `CALL_SFU_THRESHOLD` | 任意 | 既定 4。6 未満 |
| `CALL_SFU_MAX_PARTICIPANTS` | 任意 | 既定 50 |

一部だけの設定（`CALL_SFUS_JSON` がないのに Secret だけある、など）は、起動時のエラーにする。

LiveKit を有効にするときは、frontend の `CSP_CONNECT_SRC` に LiveKit の `wss://` のオリジン（例：`wss://livekit.zli.chat`）を足す。Web の CSP の `connect-src` は `https:` しか許していないので、足さないと LiveKit のシグナリング（WebSocket）がはじかれ、全員が Cloudflare に移る。

sfu-bridge の環境変数：`BACKEND_INTERNAL_URL`（クラスタ内の backend）、`SFU_BRIDGE_TOKEN`、`HEALTH_ADDR`（既定 `:8080`、`/healthz`）。イメージは `ghcr.io/zli-uoa/zoo2-sfu-bridge`（`infra/Dockerfile-sfu-bridge-deploy`）。

## プライバシー

P2P mesh では、音声は端末どうしの DTLS-SRTP で暗号化され、サーバーは中身を扱いませんでした。SFU では、Cloudflare Realtime と自前の LiveKit・sfu-bridge が、音声を復号して転送します。E2EE のチャンネルでも同じです。

## 転送量の見積もり

Opus の音声は 1 ストリームあたり約 32 kbps です。DTX が有効なので、無音のあいだはほとんど送りません。

| 条件 | 計算 | Cloudflare の送出 |
| --- | --- | --- |
| 50 人のうち同時に話すのが 2 人 | 50 人 × 2 本 × 32 kbps | 約 3.2 Mbps（1 時間で約 1.4 GB） |
| 最悪（50 人が同時に話す） | 50 人 × 49 本 × 32 kbps | 約 78 Mbps（1 時間で約 35 GB） |

Cloudflare Realtime の料金（無料枠と GB あたりの単価）は、有効にする前に最新の公式の値で確かめる。
