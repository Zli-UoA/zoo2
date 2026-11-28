# SIP電話

「設定 → 電話（SIP）」で個人用・サークル共有の回線と電話機を登録する。Web・Androidとzoo2間は通常通話と同じHTTP/SSE・WebRTC。SIPはサーバー内のJanusからPBXまでに限定し、SIP認証情報をクライアントへ渡さない。

## 経路と登録

```text
Web / Android ── 認証済みHTTP/SSE ── CF Tunnel ── zoo2 ── 内部HTTP ── Janus
Web / Android ── 公開TURN ── WebRTC ── 公開TURN ── Janus ── SIP/RTP ── 中央Asterisk
中央Asterisk ── WireGuardハブ ── 拠点Asterisk ── LAN ── ひかり電話HGW
```

同じLAN内で完結する場合はVPS・VPN・TURNを使わず、`SIP_MEDIA_MODE=lan` と空のICE設定 `[]` で直接WebRTCを利用できる。JanusはLANから到達できるIPを広告する。LAN内でもブラウザーのマイクにはHTTPSが必要。backendだけがLAN内で端末が外にある場合はVPNかTURNが必要。

拠点越しの構成で固定公開IPが必要なのはTURN/WireGuardハブのVPSだけ。中央・拠点がVPSへ外向きに接続を維持し、拠点は動的IP・NAT配下でもよい。拠点Asteriskが中央へREGISTERし、中央は登録されたcontactを利用する。中央から自宅の公開IPやHGWへREGISTERしない。HGWの認証情報は自宅にだけ置く。通常のCloudflare TunnelにはHTTP/SSEだけを通す。TURNは公開relay候補を使い、VPN/Pod CIDRへの中継やPodネットワークの公開を必要としない。

[VPS・PBX・VPNの準備](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/sip.md) はinfra側で管理する。VPSは未用意であり、このPRでは設定を準備し、自動で有効化しない。

- 個人回線は本人だけが登録・閲覧・利用・編集できる。管理者にも他人の個人回線を返さない。
- 共有回線は管理者が登録・編集し、利用者を明示する。管理者も使う場合はメンバーに含める。
- 回線に名前、外線番号（表示用）、SIP realm、認証ID・パスワードを設定する。外線番号の入力で番号取得や発信者番号変更は行わない。
- 電話機に名前と内線番号を登録する。保存済みの電話機UUIDを維持し、Asterisk Realtimeがzoo2の登録内容からendpoint・auth・AOR・回線別dialplanを読む。回線・電話機をGitOpsに登録する必要はない。物理電話機のSIPアカウントは `phone-<電話機UUID>`、認証ID/realm/passwordは所属回線の設定値。機器への設定入力は手動。
- Janusは `line-<回線UUID>`、拠点ゲートウェイは `home-<回線UUID>` として登録する。認証したendpointの回線IDでトランクを選び、Fromや表示用番号を権限判定に使わない。
- 登録済み電話機の内線番号はその電話機へ、それ以外の数字番号は同じ回線の登録済み拠点ゲートウェイへ送る。HGW着信はその回線に接続中のzooクライアントへ送る。個人回線と共有回線は別の回線ID・認証情報にする。

## 通話とクライアント

1回線を接続して待ち受けし、通話時にだけマイクを取得する。発信・応答・拒否・終了・ミュート・DTMFに対応する。発信先は数字1〜32桁と先頭の `+` のみ。任意のSIP URIやJanus命令を送信できない。

Webは発信・応答時のマイク・スピーカー設定と既存の効果音設定を利用する。通話途中の出力先変更は次の通話から反映する。チャンネル通話とのマイク利用を排他する。画面移動で接続を維持する。登録断・ログアウト・アカウント変更では破棄し、無制限の再接続や画面ごとのポーリングは行わない。ブラウザーを閉じている間の着信、保留・転送・録音・履歴は対象外。

DMと電話で音声取得・RTCPeerConnection・SDP/ICE処理・SSE受信を共有する。AndroidはComposeがHTTP/SSEを所有し、WebViewはAPK同梱のWebRTC音声処理だけを実行する。SIP.js、SIP接続先、SIPパスワード、API認証トークンをWebViewへ渡さない。DM・電話ともAndroid Telecomへ統合する。[Android版](android.md#sipdm通話とandroid-telecom) を参照。

Web・Android の SIP 通話は DM メッシュ通話と同じ送信ビットレート制御を使います。WebRTC の既定値で開始し、回線品質が悪化した場合だけ送信を 16,384 bps に抑え、回復後に既定値へ戻します。判定条件は [通話](calls.md#android-の音声ビットレート) を参照してください。

## 保存・認可・内部API

回線情報は `sip_lines`・`sip_line_members`・`sip_phones` に保存し、ElectricのShape・永続クライアントキャッシュには載せない。パスワードは回線IDをAADとするAES-256-GCMで暗号化する。鍵は全レプリカとDB復元時に維持し、再暗号化せず変更しない。一覧・更新で既存パスワードを返さず、空欄更新では保持する。応答は `private, no-store`。

`phone_sessions` はユーザー・回線の更新版・内部Janus ID・SSEリースだけを保存する。ユーザー行のロックで全レプリカ合計4接続まで。接続準備は60秒、SSEリースは45秒、10秒ごとに権限を再確認・更新する。各コマンドでも所有者・有効ユーザー・メンバー・回線の更新版を確認する。メンバー解除・回線編集後は新たな操作を拒否し、待ち受けも次の確認で切断する。切断時に登録を破棄し、開かれなかったSSEはJanusの60秒タイムアウトで回収する。遅い受信者のキューは64件で切断する。長時間のJanusイベント受信を10秒ごとにキャンセルせず保持する。

| API | 内容 |
| --- | --- |
| `GET /api/sip/lines` | 有効状態と閲覧可能な回線・電話機。パスワードなし |
| `POST /api/sip/lines` / `POST /api/sip/lines/{id}` | 登録・更新。所有区分の変更は不可 |
| `DELETE /api/sip/lines/{id}` | 管理可能な回線を削除 |
| `POST /api/sip/lines/{id}/connect` | サーバー側でSIP登録し、セッションUUID・相手UUID・ICE設定・更新版を返す |
| `POST /api/sip/lines/{id}/authorize` | 発信・応答前の権限・更新版確認 |
| `POST /api/phone/sessions/{id}/stream` | 認証付きSSE。登録・着信・SDP/ICE・終了イベントのみ |
| `POST /api/phone/sessions/{id}/command` | 発信・応答・ICE・DTMF・拒否・終了・切断 |
| `POST /api/internal/pbx/{table}/{single,multi,require}` | PBX専用Bearerで保護した読み取り専用Realtime。一般クライアントでは利用不可 |

管理PBXは `zoo-dial-contacts` 静的Gosubを含む電話イメージrevision `-3` 以降を使う。Realtime APIは短いendpoint IDと制限秒数だけを返し、静的dialplanから呼ぶ専用 `ZooDialContacts` が最大64KiBのバッファでcontactを展開し、Dialへ直接渡す。Realtimeの512バイトと通常dialplanの8192バイトの両制限を避ける。64KiBを超える場合は一部だけへ架電せず、通話全体を失敗させる。

PBX内部APIはUUIDの完全一致を起点に検索し、全endpoint列挙や書き込みを許可しない。パスワードを復号するのはauth参照だけ。Asteriskローカルのbridgeが認証ヘッダーを付け、秘密値をURLへ埋め込まない。既存外部PBXを使う場合は `SIP_PBX_CONFIG_SECRET` を設定せず、回線の認証IDでJanusが登録する。

物理電話機・拠点には回線パスワードを手動設定するため、機器の廃止・所有者変更時には回線パスワードも変更し、PBXの既存contactを失効させる。アプリメンバー解除で機器に入力したパスワードまで回収することはできない。

## 設定と検証

`SIP_ENCRYPTION_KEY`（base64 32バイト）がなければ回線機能を無効化する。`SIP_GATEWAY_URL`・`SIP_GATEWAY_API_SECRET`・`SIP_PBX_URI` は全部未設定ならゲートウェイを無効化し、一部だけなら起動時にエラーにする。管理PBXを使う場合は任意の `SIP_PBX_CONFIG_SECRET` も設定する。Debug表示で秘密値は出さない。設定はzoo2-infraのSOPS Secret `zoo2-sip` で管理する。

既存の `WEBRTC_ICE_SERVERS_JSON` に認証付きTURNが必要。既定の `SIP_MEDIA_MODE=relay` はrelay-onlyで、Janusも公開TURNへ外向きに接続する。`lan` ではWeb・Androidとも直接候補を利用し、TURNを必須にしない。UDP制限下向けにTURN/TCP・TLSを用意する。SIP接続パスは互換用の列として残すが、UI・音声処理では使わない。

APIは実PostgreSQL、メディアはChromiumの実SDP交渉・トラック解放、Androidは共通Kotlin・Robolectricで検証する。実際のVPS・SIP電話機・HGWでの発着信、TURN/VPN越しの双方向音声、発信者番号は未検証。infraの受け入れ手順で確認してから有効化する。管理ChromiumのUDP制限を回避して音声確認済みとはしない。TUIは音声機能がなく今回のAPIを利用しないため変更しない。
