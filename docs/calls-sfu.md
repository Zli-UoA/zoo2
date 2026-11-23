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
- 入室のたびに、チャンネル行のロックの下で、次の 2 つを満たすか調べる。満たせば `sfu` に切り替える。SFU が無効（`CALL_SFUS_JSON` が空）なら切り替えない。
  - 在室の人数が `CALL_SFU_THRESHOLD`（既定 4）を超えている。
  - 在室の全員が SFU に対応している。
- 空室の判定、定員の判定、切り替えの判定は、どれも入室のトランザクションの開始時刻（`transaction_timestamp()`）で在室を数える。途中で期限の切れた人を、ある判定では在室、別の判定では不在と数えないため。
- 切り替えは、入室と同じトランザクションで `mode_changed` を通知する。切り替えたあと、人数が減っても `mesh` には戻さない（空になるまで）。
- SFU に対応しているかは、クライアントが `GET /call/stream?sfu=1` で申告する。対応していないクライアント（iOS、古い Android や Web）がいる間は `mesh` のままで、定員も 6 人のまま。
- `sfu` の部屋に対応していないクライアントが入ろうとしたら、409 `CALL_SFU_UNSUPPORTED` を返す。
- iOS は SFU の部屋に入れない。そのため、`sfu` の部屋への呼び出しでは、iOS の着信（VoIP）を送らない（`apns/calls.rs`。送る時点の mode で判定する）。Web・Android への通知は送る。呼び出しのあとで部屋が `sfu` に切り替わった場合は、iOS には着信が届いていて、応答して入ろうとすると 409 になる。
- iOS がフォアグラウンドのときは、呼び出しを Electric の shape から読み、アプリが自分で着信を出す（zoo2-ios の `SystemCallController`）。呼び出しの行は Web・Android と共通なので、この経路の着信は止められず、応答すると 409 になる。iOS が SFU に対応するまでの既知の制約で、iOS 側で 409 `CALL_SFU_UNSUPPORTED` を受けたときの案内は、iOS の対応と合わせて行う。
- 定員は `capacity(mode)`。`mesh` は 6、`sfu` は `CALL_SFU_MAX_PARTICIPANTS`（既定 50、6 以上）。呼び出しの空きの計算も同じ定員を使う。SFU の定員を mesh 以上にするのは、mesh のときに予約した呼び出しの席が、切り替えで足りなくならないようにするため。
- 入室した人の SSE には、`room_state` の `mode` と `epoch` が入る。自分の入室で切り替わったときは、`mode_changed` が `room_state` より先に届くことがある。

## SFU の割り当て

`POST /api/channels/{id}/call/sfu/join`（`{ session_id, fallback?, cloudflare_only? }`）で、参加者ごとに決める。

1. `CF-Connecting-IP`（`activitypub/client_ip.rs` の信頼の条件で読む）が `lan_egress_cidrs`（LAN の出口のグローバル IP。IPv4・IPv6）に入っていれば LiveKit。
2. それ以外、IP がわからないとき、`fallback`、`cloudflare_only` のときは Cloudflare。
3. bridge が無効のときは、部屋に Cloudflare の人が 1 人でもいれば Cloudflare にする。あとから Cloudflare の人が入ったら、`sfu_reassign` で LiveKit の人に移ってもらう。LiveKit の人数は、Cloudflare のセッションを作ったあと、チャンネル行のロックを取り直して数え直す（セッションを作るあいだに LiveKit に入った人を見落とさないため）。

- 割り当てを変えたとき（fallback、やり直し）は、前の割り当ての後始末を、新しい割り当てと同じトランザクションで `call_sfu_departures` に記録する。同じ LiveKit のルームに同じ identity でつなぎ直すだけのときは記録しない（記録すると、worker の `RemoveParticipant` が新しい接続を切る）。
- 同じ LiveKit のルームへの割り当てを出し直すだけ（join の再送）のときは、割り当てを書き換えず、トラックも消さず、`track_version` も進めない。bridge が出している、その話者のトラックを残すため（LiveKit の購読が変わらないと、bridge は push し直さない）。

- ブラウザーから LAN のプライベート IP を測ること（RTT probe）はしない。公開サイトからプライベート IP へのアクセスは、ブラウザーが制限する（Private Network Access）。
- LiveKit に割り当てられても、8 秒で ICE がつながらなければ、クライアントは `fallback: true` で join し直す。
- LiveKit の JWT（HS256）の権限は次のとおり。`roomAdmin` と `roomCreate` は付けない。
  - 部屋に入れる（`roomJoin`）。部屋は 1 つに固定。
  - publish はマイクだけ（`canPublishSources: [microphone]`）。データは publish できない（`canPublishData: false`）。
  - identity は通話の `session_id`。
  - 有効期限は 60 秒（sfu-bridge のトークンは 5 分）。`sfu/join` の応答ですぐに接続するので短くしている。接続したあとの再接続には、クライアント SDK が LiveKit サーバーから受け取る更新トークンを使う。
- セルフホストの LiveKit では、`RemoveParticipant` で外しても、発行済みのトークンは無効にならない。退出・キックのあとに残る時間窓は次のとおり。運用上の制約として受け入れている。
  - backend が発行したトークン：発行から最長 60 秒、同じルームに入り直せる。
  - 接続中に LiveKit サーバーから受け取った更新トークン：その有効期限（LiveKit サーバーが決める）まで入り直せる。
  - 入り直したあとは、次に `RemoveParticipant` が呼ばれるまで外れない（presence はないので、backend は新しいトークンを発行しない）。
- Cloudflare の代理は `cf/publish`、`cf/pull`、`cf/answer` の 3 つだけ。
  - `cf/publish` は、音声の m-line が 1 本だけの offer を受け付ける。Cloudflare がその track に `errorCode` を返したら、一覧に載せずに 502 を返す（sfu-bridge の publish も同じ）。
  - push のあいだに割り当てが変わっていたら（別の Cloudflare セッションに移った、など）、409 を返し、古いセッションの track を一覧に載せない。登録は話者の presence の行をロックしてから確かめ、割り当ての変更も同じ行をロックするので、両者は直列になる。古いセッションは、割り当ての変更で記録した後始末で閉じる。
  - `cf/pull` は、backend が配るトラックの id（`{session_id}:{direct|bridge}`）で指定する。backend が同じ部屋の行に引き直すので、クライアントは Cloudflare のセッション ID や track 名を指定できない。一覧と同じく、期限の切れた（まだ消されていない）話者のトラックは pull できない。
  - app secret は backend だけが持つ。

## トラック一覧

- Cloudflare にある音声は `call_sfu_tracks` に置く。`direct` は話者のクライアントが push したもの、`bridge` は LiveKit の話者を bridge が push したもの。
- 一覧の各トラックには `rev`（公開の世代。`left(md5(cf_session_id || '/' || track_name), 12)`）が入る。同じ `id` のまま別の Cloudflare セッションで公開し直すと変わるので、pull する側（sfu-bridge）は、`rev` が変わったら前の購読を閉じて pull し直す。bridge の long-poll の `revision` にも入るので、`rev` だけが変わっても bridge は起きる。
- 話者の presence が消えると、外部キーで一緒に消える。期限が切れて、まだ消されていない presence のトラックは、一覧に入れない。
- 一覧が変わると `call_rooms.track_version` を進め、`sfu_tracks_changed`（`channel_id`・`epoch`・`version` だけ）を `pg_notify` する。一覧そのものは NOTIFY の上限（8000 バイト）を超えるので載せない。
- 通知を受けた各レプリカが DB から一覧を読み、SSE の `sfu_tracks` で配る。クライアントは、手元より古い `version` を捨てる。

## 退出したときの後始末

- `call_presence` の DELETE のトリガー（`record_call_sfu_departures`）が、SFU に割り当てた人の退出を `call_sfu_departures` に記録する。退出、sweep、キック、チャンネル削除のどの経路でも記録される。
- backend の worker（`CallSfu::start_departure_worker`）は、記録を見て次を呼び、退出した人が音声を送ることも受けることもできないようにする。
  - Cloudflare：`GET /sessions/{id}` と `tracks/close`（force）。応答の track ごとの結果を見て、閉じた mid と既に閉じていた mid（`close_track_error`）だけを成功に数える。ほかのエラーや結果のない mid があれば失敗にする。
  - LiveKit：`RemoveParticipant`。呼ぶ前に、その `session_id` の presence を主キーで読み、いま同じルームに割り当てられていれば（退出のあと、同じ session_id で入り直した）、呼ばずに済んだものとする。新しい接続を切らないため。
- worker は、短い 1 文で行を確保して（`claimed_until` を 5 分先にする。`FOR UPDATE SKIP LOCKED`）、トランザクションもロックも持たずに API を呼ぶ。確保と処理のループは SFU ごとに分かれていて（確保も `sfu_id` で絞る）、片方の API が遅くても、もう片方の後始末は待たない。SFU ごとに同時 8 件まで。worker が途中で止まっても、確保の期限のあとに別の worker が取り直す。
- 失敗したものは、15 秒あけて 5 回まで試し直す。
- LiveKit の設定がないときは、LiveKit の行を確保しない（消さずに残す）。設定を戻せば処理する。

## sfu-bridge

- 起動のたびに `POST /internal/sfu-bridge/register` で epoch（ミリ秒の時刻）を受け取る。backend は最新の epoch を `call_sfu_bridge` に記録し、それより古い bridge のトラックを消す。
- 古い epoch の bridge からの push（`/internal/sfu-bridge/cf/publish`）は 409 にして、一覧に載せない。古い bridge が止まる前に書いた行が、新しい bridge の行を上書きしないようにするため。
- bridge は 1 レプリカで、更新は `Recreate` にする（zoo2-infra）。2 つが重なって動くと、Cloudflare の話者の声が LiveKit に 2 重に届く。
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

設定は zoo2-infra の PR で入れる（[zoo2-infra の docs/calls-sfu.md](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/calls-sfu.md)）。`CALL_SFUS_JSON` が空のあいだは、今までどおり mesh だけで動く。

| 変数 | 必須 | 内容 | zoo2-infra での置き場所 |
| --- | --- | --- | --- |
| `CALL_SFUS_JSON` | 任意 | `{"cloudflare":{"api_base"?},"livekit"?:{"ws_url","api_url","lan_egress_cidrs":[…]},"bridge"?:bool}`。空なら SFU は無効 | backend の Deployment の `env`（`overlays/production/app/backend.yaml`） |
| `CF_REALTIME_APP_ID`・`CF_REALTIME_APP_SECRET` | `CALL_SFUS_JSON` があれば必須 | Cloudflare Realtime の app | Secret `zoo2-cf-realtime` のキー `CF_REALTIME_APP_ID`・`CF_REALTIME_APP_SECRET`（手で作る sops の Secret、`overlays/production/app/zoo2-cf-realtime.sops.yaml`） |
| `LIVEKIT_API_KEY`・`LIVEKIT_API_SECRET` | `livekit` があれば必須 | secret は 32 文字以上 | Secret `zoo2-livekit` のキー `LIVEKIT_API_KEY`・`LIVEKIT_API_SECRET`（OpenTofu の出力から CI が作る sops の Secret。LiveKit と共有） |
| `SFU_BRIDGE_TOKEN` | `bridge: true` なら必須 | 32 文字以上。backend と sfu-bridge で同じ値 | Secret `zoo2-sfu-bridge` のキー `SFU_BRIDGE_TOKEN`（OpenTofu の出力から CI が作る sops の Secret。sfu-bridge と共有） |
| `CALL_SFU_THRESHOLD` | 任意 | 既定 4。6 未満 | backend の Deployment の `env` |
| `CALL_SFU_MAX_PARTICIPANTS` | 任意 | 既定 50。6（mesh の定員）以上 | backend の Deployment の `env` |

起動時に次をエラーにする（一部だけの設定）。

- `CALL_SFUS_JSON` が空なのに、`CF_REALTIME_APP_ID`・`CF_REALTIME_APP_SECRET`・`LIVEKIT_API_KEY`・`LIVEKIT_API_SECRET`・`SFU_BRIDGE_TOKEN` のどれかがある。
- `CALL_SFUS_JSON` があるのに、`CF_REALTIME_APP_ID` か `CF_REALTIME_APP_SECRET` がない。
- `livekit` がないのに `LIVEKIT_API_KEY` か `LIVEKIT_API_SECRET` がある。`livekit` があるのに、どちらかがない。
- `bridge: true` でないのに `SFU_BRIDGE_TOKEN` がある。`bridge: true` なのに、`SFU_BRIDGE_TOKEN` か `livekit` がない。

URL の条件も起動時に確かめる。URL は、接続に使う HTTP クライアント（reqwest）と同じ WHATWG の解釈（`url` クレート）で読み、その host で判定する。文字列を自前で区切ると、`http://evil.example\@livekit.zoo2.svc:7880` の host を `livekit.zoo2.svc` と読み違える（WHATWG では `\` は `/` と同じ区切りで、接続先は `evil.example`）。`\` と userinfo（`user@`）を含む URL は受け付けない。

- `livekit.ws_url` は `wss://`。参加者のトークンをこの URL で送るため。`ws://` はループバック（`localhost`・`127.0.0.1`・`::1`。テスト用）だけ許す。
- `cloudflare.api_base` と `livekit.api_url` は `https://`。Cloudflare の app secret と LiveKit の管理用 JWT を Bearer で送るため。`http://` は、ループバックと、クラスタ内の Service の名前（`<service>.<namespace>.svc`・`<service>.<namespace>.svc.cluster.local`）だけ許す。本番の `livekit.api_url` は `http://livekit.zoo2.svc.cluster.local:7880`（Twirp をクラスタ内から呼ぶ。経路の扱いは backend の内部 API の平文 HTTP と同じ。zoo2-infra の docs/calls-sfu.md）。
- SFU の API を呼ぶ HTTP クライアントはリダイレクトを追わない。リダイレクト先に Bearer を送らないため。

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
