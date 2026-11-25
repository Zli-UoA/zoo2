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

- 割り当てを変えたとき（fallback、やり直し）は、前の割り当ての後始末を、新しい割り当てと同じトランザクションで `call_sfu_departures` に記録する。記録するのは前の割り当ての LiveKit の identity で、worker はその identity だけを外す。同じ LiveKit のルームへの割り当てを出し直すだけ（join の再送）のときは、identity も同じにして記録しない（記録すると、worker の `RemoveParticipant` がつないでいる接続を切る）。
- 同じ LiveKit のルームへの割り当てを出し直すだけ（join の再送）のときは、割り当てを書き換えず、トラックも消さず、`track_version` も進めない。bridge が出している、その話者のトラックを残すため（LiveKit の購読が変わらないと、bridge は push し直さない）。

- ブラウザーから LAN のプライベート IP を測ること（RTT probe）はしない。公開サイトからプライベート IP へのアクセスは、ブラウザーが制限する（Private Network Access）。
- LiveKit に割り当てられても、8 秒で ICE がつながらなければ、クライアントは `fallback: true` で join し直す。
- LiveKit の JWT（HS256）の権限は次のとおり。`roomAdmin` と `roomCreate` は付けない。
  - 部屋に入れる（`roomJoin`）。部屋は 1 つに固定。
  - publish はマイクだけ（`canPublishSources: [microphone]`）。データは publish できない（`canPublishData: false`）。
  - identity は割り当てごとの `{session_id}.{n}`（`n` は割り当てのたびに新しい値。`call_presence.livekit_identity`）。クライアントと sfu-bridge は、最初の `.` より前を `session_id` として読む（`session_id` は UUID で `.` を含まない）。同じルームへの join の再送では、同じ identity のトークンを出す。
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
- 一覧の各トラックには `rev`（公開の世代。`left(md5(cf_session_id || '/' || track_name || '/' || COALESCE(bridge_publish_id, '')), 12)`）が入る。同じ `id` のまま別の Cloudflare セッションで公開し直すと変わる（bridge は同じセッションで push し直しても変わる）ので、pull する側（sfu-bridge）は、`rev` が変わったら前の購読を閉じて pull し直す。bridge の long-poll の `revision` にも入るので、`rev` だけが変わっても bridge は起きる。
- 話者の presence が消えると、外部キーで一緒に消える。期限が切れて、まだ消されていない presence のトラックは、一覧に入れない。
- 一覧が変わると `call_rooms.track_version` を進め、`sfu_tracks_changed`（`channel_id`・`epoch`・`version` だけ）を `pg_notify` する。一覧そのものは NOTIFY の上限（8000 バイト）を超えるので載せない。
- 通知を受けた各レプリカが DB から一覧を読み、SSE の `sfu_tracks` で配る。クライアントは、手元より古い `version` を捨てる。

## 退出したときの後始末

- `call_presence` の DELETE のトリガー（`record_call_sfu_departures`）が、SFU に割り当てた人の退出を `call_sfu_departures` に記録する。退出、sweep、キック、チャンネル削除のどの経路でも記録される。
- backend の worker（`CallSfu::start_departure_worker`）は、記録を見て次を呼び、退出した人が音声を送ることも受けることもできないようにする。
  - Cloudflare：`GET /sessions/{id}` と `tracks/close`（force）。応答の track ごとの結果を見て、閉じた mid と既に閉じていた mid（`close_track_error`）だけを成功に数える。ほかのエラーや結果のない mid があれば失敗にする。
  - LiveKit：`RemoveParticipant`。外すのは、記録した割り当ての identity（`call_sfu_departures.livekit_identity`）だけ。同じ `session_id` で入り直した人は新しい identity になるので、要求が遅れて LiveKit に届いても（打ち切ったあとに LiveKit が処理した場合を含む）、新しい接続は切れない。そのため、入り直しの確認やロックは要らない。identity のない行（identity が `session_id` だったころの割り当て）は、`session_id` を外す。
- worker は、短い 1 文で行を確保して（`claimed_until` を 5 分先にする。`FOR UPDATE SKIP LOCKED`）、トランザクションもロックも持たずに API を呼ぶ。確保と処理のループは SFU ごとに分かれていて（確保も `sfu_id` で絞る）、片方の API が遅くても、もう片方の後始末は待たない。SFU ごとに同時 8 件まで。worker が途中で止まっても、確保の期限のあとに別の worker が取り直す。
- 失敗したものは、15 秒あけて試し直す。試すのは初回を含めて 5 回まで（`MAX_DEPARTURE_ATTEMPTS`）。5 回目も失敗したら、警告をログに出して行を消す。消した行は、あとで SFU が直っても処理しない（障害が長引いたときに、試し直しを待ち続けない）。
- 全件が失敗したバッチのあとも、worker は続けて次のまだ確保していない行を処理する（失敗した行は 15 秒先まで確保したままなので、選ばれない）。
- Cloudflare の操作（push・pull）が処理中のあいだに後始末が走ると、`GET` に操作の結果がまだ載っておらず、閉じ漏れる（https://developers.cloudflare.com/realtime/sfu/api/）。そのため backend は、push・pull の応答を受け取ったあとに、そのセッションがまだ割り当てのままかを確かめる。違えば（そのあいだに退出・割り当ての変更があった）、同じセッションの行を記録し直して 409 を返す。worker の次の `GET` には操作の結果が載っている。
- push・pull が失敗した、または応答がなかった（タイムアウトや 5xx。backend は 502 を返す）ときは、結果がわからない。Cloudflare では操作が済んでいる、またはこのあと済むことがある（応答を待つのをやめても、Cloudflare 側の処理は取り消されない）。そのため backend は、30 秒後（Cloudflare への要求のタイムアウト 10 秒より長い）と 5 分後に処理する行を、そのセッションについて記録する（`claimed_until` を先にする）。Cloudflare は処理時間の上限を示していないので、遅れて済んだ場合に備えて 2 回閉じ直す。worker は、その時点でセッションがまだその人の割り当てなら閉じずに行を消し、割り当てでなければ（退出・割り当ての変更のあと）閉じる。通話を続けている人のトラックは切らず、退出した人の分は、操作の結果が決まったあとの `GET` で閉じる。5 分より遅れて済んだ操作は閉じ直さない（既知の制約）。ただし、結果のわからない失敗のあと、このリポジトリのクライアント（Web・Android）はその PeerConnection を閉じて接続し直すので、遅れてできた購読の音声を受け取る相手はいない。PeerConnection を閉じない独自のクライアントでも、退出の後始末のあとに遅れて済んだ pull（応答の SDP を受け取っていない）の音声は、6 分のあいだ届かなかった（段階 1 で実際の Cloudflare で測った。下の「段階 1 の確認の結果」）。そのため、5 分より後の閉じ直しは足さない。
- クライアントは、結果のわからない pull のあと、同じセッションで pull し直さない（購読が重なることがある。https://developers.cloudflare.com/realtime/sfu/concepts/negotiation/ ）。そのセッションを捨てて、新しい PeerConnection とセッションで接続し直す。トラックごとの失敗（`errorCode`）と、Cloudflare に送る前に backend が断った要求（400・403・404）だけは、同じセッションで間隔をあけて pull し直す。409 は送ったあと（そのあいだに割り当てが変わった）にも返るので、接続し直す。Android は Kotlin が HTTP の状態を渡さないので、pull の要求の失敗はすべて接続し直す。sfu-bridge も同じで、結果のわからない pull のあとは Cloudflare のセッションを作り直す。
- LiveKit の設定がないときは、LiveKit の行を確保しない（消さずに残す）。設定を戻せば処理する。

## sfu-bridge

- 起動のたびに `POST /internal/sfu-bridge/register` で epoch（ミリ秒の時刻）を受け取る。backend は最新の epoch を `call_sfu_bridge` に記録し、それより古い bridge のトラックを消す。
- 古い epoch の bridge からの push（`/internal/sfu-bridge/cf/publish`）は 409 にして、一覧に載せない。古い bridge が止まる前に書いた行が、新しい bridge の行を上書きしないようにするため。
- bridge は 1 レプリカで、更新は `Recreate` にする（zoo2-infra）。2 つが重なって動くと、Cloudflare の話者の声が LiveKit に 2 重に届く。
- `GET /internal/sfu-bridge/rooms?bridge_epoch=…&since=…` を long-poll する。返るのは、LiveKit と Cloudflare の両方に人がいる部屋の一覧と、部屋ごとの LiveKit のトークン。変化がなければ最大 25 秒待つ。
- 部屋ごとに LiveKit に `bridge:{epoch}` として入り（hidden にはしない）、Cloudflare のセッションを 1 つ持つ。
  - LiveKit の話者の声：Cloudflare に push する（`/internal/sfu-bridge/cf/publish`、track 名は `{session_id}-bridge`）。
    bridge は push ごとに ID（`publish_id`）を付けて送り、backend は一覧の行（`call_sfu_tracks.bridge_publish_id`）に持つ。`rev` にも入れるので、同じセッションで push し直すと `rev` が変わり、クライアントは pull し直す。話者が LiveKit で公開をやめたら、bridge は `/internal/sfu-bridge/cf/close` に mid と、閉じる push の話者と ID（`pushes`）を渡す。backend は、先にその push の行を一覧から外してから（セッションと ID が同じ行だけ。同じ話者を push し直した行は ID が違うので、遅れて届いた前の close でも外れない）mid を閉じる。close が失敗しても、一覧に古い行は残らない（mid は bridge のセッションを閉じれば消える）。一覧から外せなかったときは、close もせずにエラーを返す。
    close が失敗したら、bridge は取っておいて送り直す。間隔は 5 秒から倍にしていき、1 分で止める。部屋の中継が続くあいだは諦めないので、backend が長く落ちていても、直ったあとに一覧の古い行を外せる。送り直す前に部屋の中継が止まった（Cloudflare の人がいなくなった）ときの古い行は、次にその部屋の中継を始めるとき（`/internal/sfu-bridge/cf/session`。Cloudflare のセッションを作る前）に、backend がその部屋の bridge の行を外す。`cf/session` には bridge の epoch を付け、最新でなければ（古い bridge の遅れた要求）409 にして何も外さない。外すのは、古い epoch の行と、同じ epoch で部屋の中継の世代（`room_gen`。bridge がプロセスの中で部屋の中継を作るたびに 1 ずつ増やす番号。時刻は戻りうるので使わない）が古い行だけ。同じ bridge の中で、遅れた前の中継の要求が、新しい中継の行を外すことはない。前の中継の push も、新しい中継が登録した行を上書きしない（409）。部屋で受け付けた最新の世代は `call_rooms.bridge_epoch`・`bridge_room_gen` に記録する（`cf/session` のとき）。それより古い中継の push は、話者の行がなくても登録しない（409）。記録と push の登録は、部屋の行のロックで直列にする。確認と削除は `bridge_register` と行の登録と直列にする。中継を始め直すまでのあいだ（Cloudflare の人が入ってから bridge がその部屋の中継を始めるまで）は、古い行が配られることがある。その pull は失敗する。トラックごとの失敗なら、クライアントは間隔をあけて pull し直し、中継を始め直して一覧が変わったところで新しいトラックを pull する（Cloudflare が閉じたトラックの pull にどう応えるかは、段階 4 で確かめる）。join のときに外すと、まだ動いている中継の行（join の再送、Cloudflare の人が入れ替わった直後）まで外してしまい、その話者の声が届かなくなるので、外さない。新しい中継は、LiveKit の話者を push し直して登録する。
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
- `cloudflare.api_base` と `livekit.api_url` は `https://`。Cloudflare の app secret と LiveKit の管理用 JWT を Bearer で送るため。`cloudflare.api_base` の `http://` は、ループバック（テストの mock）だけ許す。`livekit.api_url` の `http://` は、ループバックと、クラスタ内の Service の名前（`<service>.<namespace>.svc`・`<service>.<namespace>.svc.cluster.local`）だけ許す。本番の `livekit.api_url` は `http://livekit.zoo2.svc.cluster.local:7880`（Twirp をクラスタ内から呼ぶ。経路の扱いは backend の内部 API の平文 HTTP と同じ。zoo2-infra の docs/calls-sfu.md）。
- SFU の API を呼ぶ HTTP クライアントはリダイレクトを追わない。リダイレクト先に Bearer を送らないため。

LiveKit を有効にするときは、frontend の `CSP_CONNECT_SRC` に LiveKit の `wss://` のオリジン（例：`wss://livekit.zli.chat`）を足す。Web の CSP の `connect-src` は `https:` しか許していないので、足さないと LiveKit のシグナリング（WebSocket）がはじかれ、全員が Cloudflare に移る。

sfu-bridge の環境変数：`BACKEND_INTERNAL_URL`（クラスタ内の backend）、`SFU_BRIDGE_TOKEN`、`HEALTH_ADDR`（既定 `:8080`、`/healthz`）。イメージは `ghcr.io/zli-uoa/zoo2-sfu-bridge`（`infra/Dockerfile-sfu-bridge-deploy`）。

## 既知の制約

メンテナーの判断で、次は既知の制約として扱う（#671 のレビュー）。

- **iOS の着信**：iOS は SFU に対応していない。フォアグラウンドの iOS は呼び出しを Electric の shape から読んで着信を出すので、SFU の部屋への呼び出しでも着信が出て、応答すると 409 `CALL_SFU_UNSUPPORTED` になる。
- **LiveKit のトークン**：`RemoveParticipant` はセルフホストでは発行済みのトークンを失効させない。退出した人は、手元のトークンの期限（参加用は 60 秒、接続中に受け取った更新トークンはその期限）まで入り直せる。
- **クラスタ内の平文の HTTP**：次の 2 つの経路は、クラスタ内の Service 名への HTTP を許している。
  - backend から LiveKit の Twirp（`RemoveParticipant` の管理トークン）
  - sfu-bridge から backend の内部 API（`SFU_BRIDGE_TOKEN`）

## 有効にする前に確かめること

- **段階 1（Cloudflare）**：実際の Cloudflare で確かめた（下の「段階 1 の確認の結果」、Zli-UoA/zoo2-infra#213）。Preview には Cloudflare の鍵を渡す仕組みがないので、join は手元の DB から実際の Cloudflare を呼ぶ `join_latency`（`JOIN_LATENCY_REAL_CF=1`）で測った。
- **段階 3（LiveKit）**：
  - backend から LiveKit への経路が通るノード、そこにアクセスできる主体、盗聴の防ぎ方（平文を受け入れるか、TLS にするか）を確かめる。
  - セルフホストの LiveKit で、`revoke_token_ts` が効くか、退出後に手元のトークンで入り直せるかを確かめる。
  - 実際の通話で、ミュート中に受信側へ音声が届かないことを確かめる。ミュートしたまま mesh から LiveKit に移る場合と、LiveKit への publish の途中でミュートする場合の両方。結果は段階 3 の設定 PR に残す。
- **段階 4（bridge）**：sfu-bridge から backend への平文の経路を受け入れるか、暗号化するかを決める。

### 段階 1 の確認の結果

実際の Cloudflare Realtime の app で、2026-11-25 に測った。

#### 遅れて済んだ pull の音声

pion で作った送り手 2 つ（A・C）と受け手 1 つ（B）をつなぎ、B の transport が受け取るバイト数を測った（`cf-late-pull-probe`。リポジトリには入れていない）。

| 場面 | 区間 | B が受け取った量 |
| --- | --- | --- |
| 接続したまま | A を pull して受信中（基準） | 7,418 B/s |
| 接続したまま | C を pull したが、応答の SDP を B に反映しない | 7,448 B/s（増えない） |
| 接続したまま | A の mid を `tracks/close`（force）で閉じたあと | 48 B/s（RTCP だけ） |
| 退出 | 退出の後始末（B の track をすべて force で閉じる）のあと | 48 B/s |
| 退出 | そのあと C の pull が済み、応答の SDP は反映しない。30 秒ごとに 6 分 | どの区間も 48 B/s。RTP は 0。PeerConnection は最後まで connected |

- 応答の SDP を反映していない pull は、Cloudflare の一覧では `active` になるが、音声は送られてこない。退出の後始末のあとに済んだ場合も、6 分（5 分の閉じ直しより長い）のあいだ届かなかった。
- そのため、5 分より後の閉じ直しは足さない。
- 反映していない offer が残っているあいだ、そのセッションへの次の `tracks/new` は 406 `invalid_session_description` になる（renegotiation 待ち）。このリポジトリのクライアントは、結果のわからない pull のあとセッションを作り直すので、影響しない。
- `sessions/new` は本文なしで呼ぶ。本文 `{}` は 400 `decoding_error` になる。`tracks/close` の本文（`{"tracks":[{"mid":…}],"force":true}`）は通る。

#### join の遅延（実際の Cloudflare）

`join_latency` を `JOIN_LATENCY_REAL_CF=1` と `JOIN_LATENCY_WARMUP=1` で実行した。すべての join を Cloudflare に割り当てた。DB と条件は #671 の本文の `join_latency` と同じ（`call_presence` 999,999 件、手元の docker の Postgres、debug ビルド、プールの上限 10）。値は p50 / p95 / 最大（ms）。

| 条件 | join | ロック待ち | 接続待ち | SQL | 残り（Cloudflare と応答の組み立て） |
| --- | --- | --- | --- | --- | --- |
| cold・30 人・同じ部屋に同時 | 868.8 / 890.3 / 893.4 | 18.8 / 26.5 / 28.0 | 405.6 / 435.9 / 437.0 | 60.2 / 392.7 / 393.4 | 393.2 / 435.6 / 436.9 |
| 20 人・同じ部屋に同時 | 148.8 / 174.3 / 218.9 | 11.0 / 22.4 / 28.5 | 11.2 / 26.9 / 29.3 | 36.6 / 53.6 / 59.6 | 95.8 / 113.5 / 184.1 |
| 20 人・別々の部屋に同時 | 136.3 / 148.0 / 202.0 | 0.0 / 0.3 / 0.5 | 9.0 / 15.0 / 15.2 | 21.3 / 24.8 / 26.0 | 100.9 / 118.0 / 169.6 |
| 20 人・1 人ずつ | 102.7 / 139.4 / 176.3 | 0.1 / 0.1 / 0.2 | 0.0 / 0.1 / 0.1 | 8.0 / 35.1 / 36.9 | 94.1 / 113.1 / 164.4 |
| 50 人・同じ部屋に同時 | 232.8 / 269.6 / 289.7 | 14.5 / 23.1 / 25.1 | 82.0 / 117.3 / 124.7 | 48.0 / 52.9 / 54.7 | 93.7 / 123.6 / 174.9 |
| 50 人・別々の部屋に同時 | 197.1 / 216.6 / 256.5 | 0.0 / 0.5 / 4.2 | 67.5 / 83.7 / 88.9 | 25.3 / 34.5 / 39.5 | 100.8 / 126.3 / 177.9 |
| 50 人・1 人ずつ | 106.1 / 146.7 / 193.1 | 0.0 / 0.1 / 0.2 | 0.0 / 0.1 / 0.1 | 8.7 / 11.8 / 12.8 | 96.4 / 134.2 / 181.9 |

- ロック待ちは、同じ部屋に 50 人が同時に入っても p95 23 ms・最大 25 ms で、mock（Cloudflare を 100 ms 遅らせた回）と同じ程度だった。Cloudflare の呼び出しはロックの外なので、延びない。
- cold の行を除くと、残り（大半は `sessions/new`）は p50 94〜101 ms・p95 113〜134 ms・最大 184 ms。cold の行の残りは p50 393.2 ms・最大 436.9 ms。残りは join から接続待ち・SQL・handler の前の残りを引いた値なので、プールの接続を開く待ちは入らない（接続待ちの列に入る）。cold の残りが大きい理由は分けて測っていない（Cloudflare への最初の HTTPS 接続を開く時間かもしれないが、確かめていない）。
- cold の join（約 0.9 秒）は、プールが新しい接続を開く時間（推定 7 本、1 本 p50 290 ms）が大半で、#671 の mock の回と同じ傾向。本番では、プールの接続はふだんから張られている。
- 本番の DB でのロック待ちは、有効にしたあとに backend のログで見る。

## プライバシー

P2P mesh では、音声は端末どうしの DTLS-SRTP で暗号化され、サーバーは中身を扱いませんでした。SFU では、Cloudflare Realtime と自前の LiveKit・sfu-bridge が、音声を復号して転送します。E2EE のチャンネルでも同じです。

## 転送量の見積もり

Opus の音声は 1 ストリームあたり約 32 kbps です。DTX が有効なので、無音のあいだはほとんど送りません。

| 条件 | 計算 | Cloudflare の送出 |
| --- | --- | --- |
| 50 人のうち同時に話すのが 2 人 | 50 人 × 2 本 × 32 kbps | 約 3.2 Mbps（1 時間で約 1.4 GB） |
| 最悪（50 人が同時に話す） | 50 人 × 49 本 × 32 kbps | 約 78 Mbps（1 時間で約 35 GB） |

Cloudflare Realtime の料金（無料枠と GB あたりの単価）は、有効にする前に最新の公式の値で確かめる。
