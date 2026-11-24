//! 通話の SFU（docs/calls-sfu.md）の結合テスト。Cloudflare Realtime と LiveKit の API は wiremock で置き換える。
//! 環境変数で SFU を有効にするので、ほかのテストと同じバイナリには入れない。

use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path, path_regex},
};

const BRIDGE_TOKEN: &str = "bridge-token-0123456789abcdef0123456789";
const LAN_IP: &str = "203.0.113.10";

async fn request(
    app: &Router,
    actor: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
    ip: Option<&str>,
) -> anyhow::Result<(StatusCode, Value)> {
    request_with(app, actor, method, uri, body, ip, &[]).await
}

/// テスト用のヘッダー（`X-Test-Gate-*` など）を付けて呼ぶ。
async fn request_with(
    app: &Router,
    actor: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
    ip: Option<&str>,
    extra: &[(String, String)],
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Dev-User", actor)
        .header("content-type", "application/json");
    for (k, v) in extra {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let mut req = builder.body(match body {
        Some(body) => Body::from(serde_json::to_vec(&body)?),
        None => Body::empty(),
    })?;
    if let Some(ip) = ip {
        let addr: SocketAddr = format!("{ip}:5000").parse()?;
        req.extensions_mut().insert(ConnectInfo(addr));
    }
    let res = app.clone().oneshot(req).await?;
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

/// SSE で入室し、最初の `room_state` を返す。ストリームは返り値で持ち続ける（落とすと退出する）。
async fn open_stream(
    app: &Router,
    actor: &str,
    channel: &str,
    session: uuid::Uuid,
    capable: bool,
) -> anyhow::Result<(
    StatusCode,
    Option<Value>,
    Option<axum::body::BodyDataStream>,
)> {
    let sfu = if capable { "&sfu=1" } else { "" };
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/channels/{channel}/call/stream?session_id={session}{sfu}"
        ))
        .header("X-Dev-User", actor)
        .body(Body::empty())?;
    let res = app.clone().oneshot(req).await?;
    let status = res.status();
    if !status.is_success() {
        let bytes = to_bytes(res.into_body(), usize::MAX).await?;
        return Ok((status, serde_json::from_slice(&bytes).ok(), None));
    }
    let mut stream = res.into_body().into_data_stream();
    // 入室の直後は、自分の入室で起きた mode_changed が room_state より先に届くことがある。
    let mut buf = String::new();
    let data = loop {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await?
            .expect("room_state")?;
        buf.push_str(&String::from_utf8(chunk.to_vec())?);
        let found = buf.split("\n\n").find_map(|ev| {
            let is_room = ev.lines().any(|l| l.trim() == "event: room_state");
            ev.lines()
                .find_map(|l| l.strip_prefix("data:"))
                .filter(|_| is_room)
                .map(|d| d.trim().to_string())
        });
        if let Some(d) = found {
            break Some(serde_json::from_str::<Value>(&d)?);
        }
    };
    Ok((status, data, Some(stream)))
}

async fn count(db: &mut toasty::Db, sql: &str, bind: &str) -> anyhow::Result<i64> {
    Ok(text(db, sql, bind).await?.parse()?)
}

/// Cloudflare の `sessions/new` の代わり。要求ごとに新しいセッション ID を返す（ID が同じだと、
/// 別の人のセッションを閉じても区別できない）。
struct NewSession;

impl wiremock::Respond for NewSession {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        ResponseTemplate::new(201)
            .set_body_json(json!({ "sessionId": format!("cf-{}", uuid::Uuid::new_v4()) }))
    }
}

/// 応答を遅らせる `sessions/new`（セッションを作るあいだに、ほかの人が入る場合）。
struct SlowNewSession(Duration);

impl wiremock::Respond for SlowNewSession {
    fn respond(&self, req: &wiremock::Request) -> ResponseTemplate {
        NewSession.respond(req).set_delay(self.0)
    }
}

/// Cloudflare の `tracks/new`・`tracks/close` の代わり。要求した track ごとに結果を返す。mid が "9" の
/// track だけは失敗（`internal_error`）にする。`delay` は応答を遅らせる（push のあいだの割り当ての変更）。
struct EchoTracks {
    answer: bool,
    delay: Option<Duration>,
}

impl wiremock::Respond for EchoTracks {
    fn respond(&self, req: &wiremock::Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let tracks: Vec<Value> = body["tracks"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|t| {
                let mid = t["mid"].as_str().unwrap_or("0").to_string();
                if mid == "9" {
                    json!({"mid": mid, "errorCode": "internal_error", "errorDescription": "Backend error"})
                } else {
                    json!({"mid": mid, "trackName": t["trackName"]})
                }
            })
            .collect();
        let mut res = json!({"tracks": tracks, "requiresImmediateRenegotiation": false});
        if self.answer {
            res["sessionDescription"] = json!({"type": "answer", "sdp": "v=0\r\n"});
        }
        let res = ResponseTemplate::new(200).set_body_json(res);
        match self.delay {
            Some(d) => res.set_delay(d),
            None => res,
        }
    }
}

/// LiveKit の mock が受けた、`room` と `identity` を指定した RemoveParticipant の数。`skip` 件目より後だけを見る。
async fn remove_participant_calls(
    lk: &MockServer,
    skip: usize,
    room: &str,
    identity: &str,
) -> usize {
    let expected = json!({ "room": room, "identity": identity });
    lk.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .skip(skip)
        .filter(|r| {
            r.method == wiremock::http::Method::POST
                && r.url.path() == "/twirp/livekit.RoomService/RemoveParticipant"
                && serde_json::from_slice::<Value>(&r.body).is_ok_and(|b| b == expected)
        })
        .count()
}

/// Cloudflare の mock が受けた、`cf_session` の `tracks/close`。`skip` 件目より後だけを見る。
async fn close_calls(cf: &MockServer, skip: usize, cf_session: &str) -> Vec<Value> {
    let close_path = format!("/apps/app/sessions/{cf_session}/tracks/close");
    cf.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .skip(skip)
        .filter(|r| r.method == wiremock::http::Method::PUT && r.url.path() == close_path)
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .collect()
}

/// sfu-bridge の内部 API を呼ぶ。`tokio::spawn` に渡せるよう、Router を値で受け取る。
async fn bridge_call(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    bridge_call_with(app, method, uri, body, Vec::new()).await
}

async fn bridge_call_with(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    extra: Vec<(String, String)>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {BRIDGE_TOKEN}"))
        .header("content-type", "application/json");
    for (k, v) in &extra {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let req = builder
        .body(match body {
            Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
            None => Body::empty(),
        })
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
    )
}

/// テスト用の一時停止（backend の `X-Test-Gate-*`）。`headers` を要求に付け、`reached` で到着を待ち、
/// `release` で先に進める。
struct Gate {
    reached: String,
    wait: String,
}

impl Gate {
    fn new() -> Self {
        let id = uuid::Uuid::new_v4();
        Self {
            reached: format!("sfu-reached-{id}"),
            wait: format!("sfu-wait-{id}"),
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("X-Test-Gate-Reached".into(), self.reached.clone()),
            ("X-Test-Gate-Wait".into(), self.wait.clone()),
            ("X-Test-Gate-Wait-Ms".into(), "20000".into()),
        ]
    }

    async fn reached(&self) -> anyhow::Result<()> {
        tokio::time::timeout(
            Duration::from_secs(10),
            backend::routes::messages::test_gate(&self.reached).notified(),
        )
        .await?;
        Ok(())
    }

    fn release(&self) {
        backend::routes::messages::test_gate(&self.wait).notify_one();
    }
}

/// LiveKit のルームの、接続中の identity を持つ mock。`RemoveParticipant` は受けたときには反映せず
/// （応答は `delay` だけ保留する）、`apply_late` を呼んだときに反映する。backend が打ち切ったあとに LiveKit が
/// 古い要求を処理する状況を再現する。
#[derive(Clone, Default)]
struct LiveKitRoomState {
    connected: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    received: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl LiveKitRoomState {
    fn connect(&self, identity: &str) {
        self.connected.lock().unwrap().insert(identity.to_string());
    }

    fn is_connected(&self, identity: &str) -> bool {
        self.connected.lock().unwrap().contains(identity)
    }

    /// 受けた RemoveParticipant を、いま反映する（遅れて処理された）。
    fn apply_late(&self) {
        let received = std::mem::take(&mut *self.received.lock().unwrap());
        let mut connected = self.connected.lock().unwrap();
        for identity in received {
            connected.remove(&identity);
        }
    }
}

struct LateRemove {
    state: LiveKitRoomState,
    delay: Duration,
}

impl wiremock::Respond for LateRemove {
    fn respond(&self, req: &wiremock::Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        if let Some(identity) = body["identity"].as_str() {
            self.state
                .received
                .lock()
                .unwrap()
                .push(identity.to_string());
        }
        ResponseTemplate::new(200)
            .set_body_json(json!({}))
            .set_delay(self.delay)
    }
}

/// LiveKit の JWT の `sub`（identity）。
fn token_sub(token: &str) -> String {
    use base64::Engine;
    let payload = token.split('.').nth(1).expect("jwt payload");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("base64url");
    let claims: Value = serde_json::from_slice(&bytes).expect("claims");
    claims["sub"].as_str().expect("sub").to_string()
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap_or_default().len()
}

async fn text(db: &mut toasty::Db, sql: &str, bind: &str) -> anyhow::Result<String> {
    let rows = toasty::sql::query(format!("SELECT ({sql})::text"))
        .bind(bind.to_string())
        .exec(db)
        .await?;
    let toasty::stmt::Value::Record(rec) = rows.into_iter().next().expect("row") else {
        anyhow::bail!("not a record");
    };
    let Some(toasty::stmt::Value::String(s)) = rec.fields.into_iter().next() else {
        anyhow::bail!("not a string");
    };
    Ok(s)
}

#[tokio::test]
async fn sfu_mode_switch_assignment_and_cleanup() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let cf = MockServer::start().await;
    let lk = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/apps/app/sessions/new$"))
        .respond_with(NewSession)
        .mount(&cf)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/apps/app/sessions/[^/]+/tracks/new$"))
        .respond_with(EchoTracks {
            answer: true,
            delay: None,
        })
        .mount(&cf)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/apps/app/sessions/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tracks": [{"mid": "0", "status": "active"}]
        })))
        .mount(&cf)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"^/apps/app/sessions/[^/]+/tracks/close$"))
        .respond_with(EchoTracks {
            answer: false,
            delay: None,
        })
        .mount(&cf)
        .await;
    Mock::given(method("POST"))
        .and(path("/twirp/livekit.RoomService/RemoveParticipant"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&lk)
        .await;

    let sfus = json!({
        "cloudflare": {"api_base": cf.uri()},
        "livekit": {
            "ws_url": "wss://livekit.example",
            "api_url": lk.uri(),
            "lan_egress_cidrs": [format!("{LAN_IP}/32")]
        },
        "bridge": true
    });
    // SAFETY: このテストバイナリは、このテストだけで、env を読むのは下の AppState::new の前。
    unsafe {
        std::env::set_var("CALL_SFUS_JSON", sfus.to_string());
        std::env::set_var("CF_REALTIME_APP_ID", "app");
        std::env::set_var("CF_REALTIME_APP_SECRET", "secret");
        std::env::set_var("LIVEKIT_API_KEY", "key");
        std::env::set_var("LIVEKIT_API_SECRET", "0123456789abcdef0123456789abcdef");
        std::env::set_var("SFU_BRIDGE_TOKEN", BRIDGE_TOKEN);
        // .env の値に左右されないよう、閾値と定員を固定する。
        std::env::set_var("CALL_SFU_THRESHOLD", "4");
        std::env::set_var("CALL_SFU_MAX_PARTICIPANTS", "50");
        std::env::remove_var("ACTIVITYPUB_CLOUDFLARE_TUNNEL");
    }

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    assert!(state.call_sfu.is_some());
    let app = create_router(state.clone());
    // LISTEN の接続を待つ（ほかのテストと同じく、待てなければ手で有効にする）。
    for _ in 0..50 {
        if state.call_registry.is_listening() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    state.call_registry.set_db(db.clone()).await;

    let users: Vec<String> = (0..7).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let channel = uuid::Uuid::new_v4().to_string();
    for (i, id) in users.iter().enumerate() {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
        )
        .bind(id.as_str())
        .bind(format!("User{i}"))
        .exec(&mut db)
        .await?;
    }
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
    )
    .bind(channel.as_str())
    .bind(users[0].as_str())
    .exec(&mut db)
    .await?;

    // 4 人までは mesh。
    let sessions: Vec<uuid::Uuid> = (0..7).map(|_| uuid::Uuid::new_v4()).collect();
    let mut streams = Vec::new();
    for i in 0..4 {
        let (status, room, stream) =
            open_stream(&app, &users[i], &channel, sessions[i], true).await?;
        assert_eq!(status, StatusCode::OK, "join {i}");
        let room = room.expect("room_state");
        assert_eq!(room["mode"], "mesh", "join {i}");
        streams.push(stream);
    }
    let epoch_mesh = count(
        &mut db,
        "SELECT epoch FROM call_rooms WHERE channel_id = $1",
        &channel,
    )
    .await?;

    // mesh のあいだは sfu/join できない。
    let (status, _) = request(
        &app,
        &users[0],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/join"),
        Some(json!({"session_id": sessions[0]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);

    // 5 人目で SFU に切り替わる。
    let (status, room, stream) = open_stream(&app, &users[4], &channel, sessions[4], true).await?;
    assert_eq!(status, StatusCode::OK);
    let room = room.expect("room_state");
    assert_eq!(room["mode"], "sfu");
    assert_eq!(room["max_participants"], 50);
    assert_eq!(room["epoch"].as_i64(), Some(epoch_mesh));
    streams.push(stream);

    // SFU に対応していないクライアントは入れない。
    let (status, body, _) = open_stream(&app, &users[5], &channel, sessions[5], false).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body.unwrap()["code"], "CALL_SFU_UNSUPPORTED");

    // 状態の API も SFU の定員を返す。
    let (status, status_body) = request(
        &app,
        &users[6],
        "GET",
        &format!("/api/channels/{channel}/call"),
        None,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(status_body["mode"], "sfu");
    assert_eq!(status_body["max_participants"], 50);

    // LAN の出口から来た人は LiveKit。
    let (status, joined) = request(
        &app,
        &users[0],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/join"),
        Some(json!({"session_id": sessions[0]})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "livekit");
    assert_eq!(joined["url"], "wss://livekit.example");
    assert!(
        joined["token"]
            .as_str()
            .is_some_and(|t| t.split('.').count() == 3)
    );
    // identity は割り当てごとの `{session_id}.{n}`。
    let lk_identity = token_sub(joined["token"].as_str().unwrap());
    assert!(
        lk_identity.starts_with(&format!("{}.", sessions[0])),
        "{lk_identity}"
    );
    // 同じ LiveKit のルームにつなぎ直すだけなら、新しい接続を切らないよう、後始末を記録しない。
    // worker は入り直した人の行を、RemoveParticipant を呼ばずに消すので、worker が動いていると、誤って
    // 記録した行も見えなくなる。worker の確保を止めてから join し、行が作られないことを確かめる。
    // 戻ったときには、確保の文を実行中の worker はいない（実行中の確保が終わるまで待つ）。
    backend::call_sfu::pause_departure_worker_for_test(true).await;
    let (status, joined) = request(
        &app,
        &users[0],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/join"),
        Some(json!({"session_id": sessions[0]})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "livekit");
    // 同じ割り当ての出し直しなので、identity も同じ。
    assert_eq!(token_sub(joined["token"].as_str().unwrap()), lk_identity);
    assert_eq!(
        count(
            &mut db,
            "SELECT count(*) FROM call_sfu_departures WHERE channel_id = $1",
            &channel
        )
        .await?,
        0,
        "re-joining the same LiveKit room recorded a departure"
    );
    backend::call_sfu::pause_departure_worker_for_test(false).await;
    // worker を戻して起こし、このルームと identity への RemoveParticipant が届いていないことも確かめる。
    let sfu = state.call_sfu.clone().expect("sfu");
    let lk_room = format!("call-{channel}-{epoch_mesh}");
    sfu.departures_wake.notify_one();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        remove_participant_calls(&lk, 0, &lk_room, &lk_identity).await,
        0,
        "re-joining the same LiveKit room must not remove the participant"
    );

    // 同じ session_id の、前の割り当ての後始末が残っていても（退出してすぐ、同じルームに入り直した）、
    // worker が外すのは前の identity だけで、いまの identity の接続は外さない。
    let old_identity = format!("{}.0000000000000000", sessions[0]);
    toasty::sql::statement(
        "INSERT INTO call_sfu_departures (channel_id, session_id, sfu_id, sfu_room, livekit_identity)
         VALUES ($1, $2::uuid, 'livekit', $3, $4)",
    )
    .bind(channel.as_str())
    .bind(sessions[0].to_string())
    .bind(lk_room.as_str())
    .bind(old_identity.as_str())
    .exec(&mut db)
    .await?;
    sfu.departures_wake.notify_one();
    let mut drained = false;
    for _ in 0..40 {
        if count(
            &mut db,
            "SELECT count(*) FROM call_sfu_departures WHERE channel_id = $1",
            &channel,
        )
        .await?
            == 0
        {
            drained = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(drained, "the departure worker did not process the row");
    assert_eq!(
        remove_participant_calls(&lk, 0, &lk_room, &old_identity).await,
        1,
        "the worker did not remove the old identity"
    );
    assert_eq!(
        remove_participant_calls(&lk, 0, &lk_room, &lk_identity).await,
        0,
        "the worker removed the identity of the current assignment"
    );

    // fallback なら Cloudflare。
    let (status, joined) = request(
        &app,
        &users[1],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/join"),
        Some(json!({"session_id": sessions[1], "fallback": true})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "cloudflare");

    // IP がわからない人も Cloudflare。
    let (status, joined) = request(
        &app,
        &users[2],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/join"),
        Some(json!({"session_id": sessions[2]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "cloudflare");
    // Cloudflare のセッションは参加者ごとに別。
    let cf_of = |sid: uuid::Uuid| {
        format!(
            "SELECT cf_session_id FROM call_presence WHERE channel_id = $1 AND session_id = '{sid}'"
        )
    };
    let cf_tracks_of = |sid: uuid::Uuid| {
        format!(
            "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1 AND session_id = '{sid}'"
        )
    };
    assert_ne!(
        text(&mut db, &cf_of(sessions[1]), &channel).await?,
        text(&mut db, &cf_of(sessions[2]), &channel).await?
    );

    // 他人のセッションでは join できない。
    let (status, _) = request(
        &app,
        &users[3],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/join"),
        Some(json!({"session_id": sessions[2]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 音声 1 本の offer だけ push できる。
    let offer =
        json!({"type": "offer", "sdp": "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\n"});
    let video =
        json!({"type": "offer", "sdp": "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:0\r\n"});
    let (status, _) = request(
        &app,
        &users[1],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/cf/publish"),
        Some(json!({"session_id": sessions[1], "offer": video})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, published) = request(
        &app,
        &users[1],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/cf/publish"),
        Some(json!({"session_id": sessions[1], "offer": offer})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{published}");
    assert_eq!(published["sessionDescription"]["type"], "answer");
    // LiveKit の人は Cloudflare の代理を使えない。
    let (status, _) = request(
        &app,
        &users[0],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/cf/publish"),
        Some(json!({"session_id": sessions[0], "offer": offer})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);

    let track_id = format!("{}:direct", sessions[1]);
    // ほかの人の声は pull できる。自分の声だけなら何もない。存在しない id は無視する。
    let (status, _) = request(
        &app,
        &users[2],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/cf/pull"),
        Some(json!({"session_id": sessions[2], "track_ids": [track_id]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    // pull は、聞く人のセッションに、話す人のセッションと track 名を指定して出す。
    let subscriber_cf = text(&mut db, &cf_of(sessions[2]), &channel).await?;
    let publisher_cf = text(&mut db, &cf_of(sessions[1]), &channel).await?;
    let pull_path = format!("/apps/app/sessions/{subscriber_cf}/tracks/new");
    let pulled = cf
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::POST && r.url.path() == pull_path)
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .any(|b| {
            b["tracks"]
                == json!([{
                    "location": "remote",
                    "sessionId": publisher_cf,
                    "trackName": sessions[1].to_string(),
                }])
        });
    assert!(
        pulled,
        "the pull did not target {subscriber_cf} with the track of {publisher_cf}"
    );
    let (status, _) = request(
        &app,
        &users[1],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/cf/pull"),
        Some(json!({"session_id": sessions[1], "track_ids": [track_id]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = request(
        &app,
        &users[2],
        "POST",
        &format!("/api/channels/{channel}/call/sfu/cf/pull"),
        Some(json!({"session_id": sessions[2], "track_ids": ["not-a-track"]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // push の応答を受けたあと、登録の前に割り当てが変わったら（fallback で新しい Cloudflare セッションへ）、
    // その push は 409 で、古いセッションのトラックは一覧に載らない。順序は backend の test gate で固定する
    // （Cloudflare の応答のあと、register_track の前で止める）。
    let publish_uri = format!("/api/channels/{channel}/call/sfu/cf/publish");
    let join_uri = format!("/api/channels/{channel}/call/sfu/join");
    let old_cf2 = text(&mut db, &cf_of(sessions[2]), &channel).await?;
    let publish_path = format!("/apps/app/sessions/{old_cf2}/tracks/new");
    // 同じパスには先の pull も届いているので、ここから後の要求だけを見る。
    let before_publish = request_count(&cf).await;
    let gate = Gate::new();
    let pending = {
        let app = app.clone();
        let user = users[2].clone();
        let uri = publish_uri.clone();
        let body = json!({"session_id": sessions[2], "offer": offer});
        let headers = gate.headers();
        tokio::spawn(async move {
            request_with(&app, &user, "POST", &uri, Some(body), None, &headers).await
        })
    };
    gate.reached().await?;
    // 止まっている publish は、古いセッションへの push（location: local）を済ませている。
    let pushed_old = cf
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .skip(before_publish)
        .filter(|r| r.method == wiremock::http::Method::POST && r.url.path() == publish_path)
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .any(|b| b["tracks"][0]["location"] == "local");
    assert!(pushed_old, "the paused publish did not push to {old_cf2}");
    let (status, joined) = request(
        &app,
        &users[2],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[2], "fallback": true})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "cloudflare");
    let new_cf2 = text(&mut db, &cf_of(sessions[2]), &channel).await?;
    assert_ne!(new_cf2, old_cf2);
    assert!(
        !pending.is_finished(),
        "the publish did not wait at the gate"
    );
    gate.release();
    let (status, body) = pending.await??;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        count(&mut db, &cf_tracks_of(sessions[2]), &channel).await?,
        0,
        "a publish to the old Cloudflare session was registered"
    );

    // push が先に終わってから割り当てが変わったら、古いトラックは消える。
    let (status, published) = request(
        &app,
        &users[2],
        "POST",
        &publish_uri,
        Some(json!({"session_id": sessions[2], "offer": offer})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{published}");
    assert_eq!(
        text(
            &mut db,
            &format!(
                "SELECT cf_session_id FROM call_sfu_tracks WHERE channel_id = $1 AND session_id = '{}'",
                sessions[2]
            ),
            &channel
        )
        .await?,
        new_cf2
    );
    let (status, joined) = request(
        &app,
        &users[2],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[2], "fallback": true})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(
        count(&mut db, &cf_tracks_of(sessions[2]), &channel).await?,
        0,
        "the track of the old Cloudflare session was not deleted"
    );

    // bridge：トークンがなければ 401。LiveKit と Cloudflare の両方に人がいる部屋が返る。
    let (status, _) = request(
        &app,
        &users[0],
        "POST",
        "/internal/sfu-bridge/register",
        None,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let bridge = |method: &str, uri: &str, body: Option<Value>| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {BRIDGE_TOKEN}"))
            .header("content-type", "application/json");
        let _ = &mut req;
        let req = req
            .body(match body {
                Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
                None => Body::empty(),
            })
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let (status, reg) = bridge("POST", "/internal/sfu-bridge/register", None).await;
    assert_eq!(status, StatusCode::OK);
    let bridge_epoch = reg["bridge_epoch"].as_i64().unwrap();
    let (status, rooms) = bridge(
        "GET",
        &format!("/internal/sfu-bridge/rooms?bridge_epoch={bridge_epoch}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room = rooms["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel_id"] == channel.as_str())
        .expect("room needs a bridge")
        .clone();
    assert_eq!(room["livekit_sessions"], json!([sessions[0]]));
    assert_eq!(room["cloudflare_tracks"][0]["id"], track_id);
    // 公開の世代（rev）は、Cloudflare のセッションと track 名から作る。
    let rev_sql = format!(
        "SELECT left(md5(cf_session_id || '/' || track_name), 12) FROM call_sfu_tracks
         WHERE channel_id = $1 AND session_id = '{}' AND source = 'direct'",
        sessions[1]
    );
    let rev1 = text(&mut db, &rev_sql, &channel).await?;
    assert_eq!(room["cloudflare_tracks"][0]["rev"], rev1.as_str());
    // long-poll：同じ revision なら、変化があるまで応答を保留する。
    let since = rooms["revision"].as_str().unwrap().to_string();
    let mut held = {
        let app = app.clone();
        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/internal/sfu-bridge/rooms?bridge_epoch={bridge_epoch}&since={since}"
            ))
            .header("authorization", format!("Bearer {BRIDGE_TOKEN}"))
            .body(Body::empty())?;
        tokio::spawn(async move {
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        })
    };
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !held.is_finished(),
        "the long-poll returned without a change"
    );
    // 同じ id のまま、別の Cloudflare セッションで公開し直すと、rev と revision が変わる（bridge が pull し直す）。
    let (status, joined) = request(
        &app,
        &users[1],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[1], "fallback": true})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    let (status, published) = request(
        &app,
        &users[1],
        "POST",
        &publish_uri,
        Some(json!({"session_id": sessions[1], "offer": offer})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{published}");
    let rev2 = text(&mut db, &rev_sql, &channel).await?;
    assert_ne!(rev2, rev1);
    // 保留していた long-poll が、新しい revision で返る（期限の 25 秒より前に）。
    // （割り当ての変更でトラックが消えた時点で返ることもあるので、中身は次の読み取りで確かめる。）
    let (status, woke) = tokio::time::timeout(Duration::from_secs(10), &mut held).await??;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(woke["revision"].as_str(), Some(since.as_str()));
    let (status, rooms) = bridge(
        "GET",
        &format!("/internal/sfu-bridge/rooms?bridge_epoch={bridge_epoch}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(rooms["revision"].as_str(), Some(since.as_str()));
    let room = rooms["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel_id"] == channel.as_str())
        .expect("room needs a bridge")
        .clone();
    assert_eq!(room["cloudflare_tracks"][0]["id"], track_id);
    assert_eq!(room["cloudflare_tracks"][0]["rev"], rev2.as_str());
    // LiveKit の人の声を、bridge が Cloudflare に push する。
    let two = json!({"type": "offer", "sdp": "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=audio 9 X 111\r\na=mid:1\r\n"});
    let (status, _) = bridge(
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": bridge_epoch, "cf_session_id": "cf-bridge",
            "speaker_session_id": sessions[0], "offer": two, "mid": "1"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Cloudflare が track ごとに失敗を返したら、一覧に載せない。
    let failing = json!({"type": "offer", "sdp": "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=audio 9 X 111\r\na=mid:9\r\n"});
    let (status, _) = bridge(
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": bridge_epoch, "cf_session_id": "cf-bridge-2",
            "speaker_session_id": sessions[0], "offer": failing, "mid": "9"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        text(
            &mut db,
            "SELECT cf_session_id FROM call_sfu_tracks WHERE channel_id = $1 AND source = 'bridge'",
            &channel
        )
        .await?,
        "cf-bridge"
    );
    // Cloudflare の人は LiveKit の話者をできない。
    let (status, _) = bridge(
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": bridge_epoch, "cf_session_id": "cf-bridge",
            "speaker_session_id": sessions[1], "offer": two, "mid": "1"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        count(
            &mut db,
            "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1",
            &channel
        )
        .await?,
        2
    );
    // 中継中の話者が、同じ LiveKit のルームへの join を再送しても、bridge のトラックは残り、版も進まない。
    let version_sql = "SELECT track_version FROM call_rooms WHERE channel_id = $1";
    let version_before = count(&mut db, version_sql, &channel).await?;
    let (status, joined) = request(
        &app,
        &users[0],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[0]})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "livekit");
    assert_eq!(
        count(&mut db, &cf_tracks_of(sessions[0]), &channel).await?,
        1,
        "re-joining the same LiveKit room deleted the bridge track"
    );
    assert_eq!(count(&mut db, version_sql, &channel).await?, version_before);
    // join の応答の一覧にも rev が入る。
    let tracks = joined["tracks"].as_array().expect("tracks");
    assert_eq!(tracks.len(), 2, "{joined}");
    assert!(
        tracks
            .iter()
            .all(|t| t["rev"].as_str().is_some_and(|r| r.len() == 12)),
        "{joined}"
    );
    // 新しい bridge が登録すると、前の bridge のトラックは消える。
    let (_, reg2) = bridge("POST", "/internal/sfu-bridge/register", None).await;
    assert!(reg2["bridge_epoch"].as_i64().unwrap() > bridge_epoch);
    assert_eq!(
        count(
            &mut db,
            "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1 AND source = 'bridge'",
            &channel
        )
        .await?,
        0
    );
    // 前の bridge がまだ動いていても、その push は一覧に載らない。
    let (status, stale) = bridge(
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": bridge_epoch, "cf_session_id": "cf-bridge",
            "speaker_session_id": sessions[0],
            "offer": {"type": "offer", "sdp": "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=audio 9 X 111\r\na=mid:1\r\n"},
            "mid": "1"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(
        count(
            &mut db,
            "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1 AND source = 'bridge'",
            &channel
        )
        .await?,
        0
    );

    // 古い bridge の push と、新しい bridge の登録が重なったとき（register_track の FOR SHARE と、
    // bridge_register の更新で直列になる）。
    let bridge_tracks_sql =
        "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1 AND source = 'bridge'";
    let epoch2 = reg2["bridge_epoch"].as_i64().unwrap();
    let speaker_offer = json!({"type": "offer", "sdp": "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=audio 9 X 111\r\na=mid:1\r\n"});
    // (a) Cloudflare の応答を待つあいだに新しい bridge が登録したら、その push は 409 で、一覧に残らない。
    let slow_cf = format!("cf-bridge-slow-{}", uuid::Uuid::new_v4());
    let slow_path = format!("/apps/app/sessions/{slow_cf}/tracks/new");
    Mock::given(method("POST"))
        .and(path(slow_path.as_str()))
        .respond_with(EchoTracks {
            answer: true,
            delay: Some(Duration::from_millis(1500)),
        })
        .with_priority(1)
        .mount(&cf)
        .await;
    let pending = tokio::spawn(bridge_call(
        app.clone(),
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": epoch2, "cf_session_id": slow_cf,
            "speaker_session_id": sessions[0], "offer": speaker_offer, "mid": "1"
        })),
    ));
    let mut reached = false;
    for _ in 0..40 {
        reached = cf
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|r| r.url.path() == slow_path);
        if reached {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        reached,
        "the delayed bridge publish did not reach Cloudflare"
    );
    let (status, reg3) = bridge("POST", "/internal/sfu-bridge/register", None).await;
    assert_eq!(status, StatusCode::OK);
    let epoch3 = reg3["bridge_epoch"].as_i64().unwrap();
    assert!(epoch3 > epoch2);
    let (status, body) = pending.await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(count(&mut db, bridge_tracks_sql, &channel).await?, 0);

    // (b) push の登録が先に終わったら、あとの登録がそのトラックを消す。
    let (status, body) = bridge(
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": epoch3, "cf_session_id": "cf-bridge-3",
            "speaker_session_id": sessions[0], "offer": speaker_offer, "mid": "1"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&mut db, bridge_tracks_sql, &channel).await?, 1);
    let (_, reg4) = bridge("POST", "/internal/sfu-bridge/register", None).await;
    let epoch4 = reg4["bridge_epoch"].as_i64().unwrap();
    assert!(epoch4 > epoch3);
    assert_eq!(count(&mut db, bridge_tracks_sql, &channel).await?, 0);

    // (b') 本番の register_track が、epoch を確かめて FOR SHARE を持ったまま、書く前で止まっているあいだは、
    // 新しい bridge の登録は終わらない。publish の commit のあと、登録はそのトラックを消す。
    let gate = Gate::new();
    let mut publishing = tokio::spawn(bridge_call_with(
        app.clone(),
        "POST",
        "/internal/sfu-bridge/cf/publish",
        Some(json!({
            "channel_id": channel, "bridge_epoch": epoch4, "cf_session_id": "cf-bridge-4",
            "speaker_session_id": sessions[0], "offer": speaker_offer, "mid": "1"
        })),
        gate.headers(),
    ));
    gate.reached().await?;
    let mut registering = tokio::spawn(bridge_call(
        app.clone(),
        "POST",
        "/internal/sfu-bridge/register",
        None,
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !registering.is_finished(),
        "register did not wait for the FOR SHARE lock of register_track"
    );
    assert_eq!(count(&mut db, bridge_tracks_sql, &channel).await?, 0);
    gate.release();
    let (status, body) = tokio::time::timeout(Duration::from_secs(10), &mut publishing).await??;
    // epoch を確かめた時点では最新だったので、publish は成功する。
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, reg5) = tokio::time::timeout(Duration::from_secs(10), &mut registering).await??;
    assert_eq!(status, StatusCode::OK);
    assert!(reg5["bridge_epoch"].as_i64().unwrap() > epoch4);
    // 登録は publish の commit を待ってから消すので、epoch4 のトラックは残らない。
    assert_eq!(count(&mut db, bridge_tracks_sql, &channel).await?, 0);

    // Cloudflare の人が退出すると、トラックが消え、worker が Cloudflare のセッションを閉じる。
    let left_cf_session = text(
        &mut db,
        &format!(
            "SELECT cf_session_id FROM call_presence WHERE channel_id = $1 AND session_id = '{}'",
            sessions[1]
        ),
        &channel,
    )
    .await?;
    // 残る人のセッション。退出の後始末で閉じてはいけない。
    let staying_cf_session = text(&mut db, &cf_of(sessions[2]), &channel).await?;
    assert_ne!(staying_cf_session, left_cf_session);
    // leave より後に届いた要求だけを見る。
    let cf_before_leave = request_count(&cf).await;
    let (status, _) = request(
        &app,
        &users[1],
        "POST",
        &format!("/api/channels/{channel}/call/leave"),
        Some(json!({"session_id": sessions[1]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        count(
            &mut db,
            "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1",
            &channel
        )
        .await?,
        0
    );
    let mut closed = false;
    for _ in 0..60 {
        let left = count(
            &mut db,
            "SELECT count(*) FROM call_sfu_departures WHERE channel_id = $1",
            &channel,
        )
        .await?;
        if left == 0 {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        closed,
        "departure worker did not close the Cloudflare session"
    );
    // 退出した人のセッションを、force で閉じたこと（共有 DB のほかの退出への要求と区別する）。
    let closed_left = close_calls(&cf, cf_before_leave, &left_cf_session)
        .await
        .into_iter()
        .any(|b| b["force"] == true && b["tracks"] == json!([{"mid": "0"}]));
    assert!(
        closed_left,
        "the departed Cloudflare session {left_cf_session} was not force-closed"
    );
    assert!(
        close_calls(&cf, cf_before_leave, &staying_cf_session)
            .await
            .is_empty(),
        "the Cloudflare session of a participant who stayed was closed"
    );

    // LiveKit の人が退出すると、RemoveParticipant を呼ぶ。leave より後に届いた要求だけを見る。
    let lk_before_leave = request_count(&lk).await;
    let (status, _) = request(
        &app,
        &users[0],
        "POST",
        &format!("/api/channels/{channel}/call/leave"),
        Some(json!({"session_id": sessions[0]})),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    // 退出した人のルームと identity を指定して、RemoveParticipant を呼んだこと。
    let mut removed = false;
    for _ in 0..60 {
        removed = remove_participant_calls(&lk, lk_before_leave, &lk_room, &lk_identity).await > 0;
        if removed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(removed, "LiveKit RemoveParticipant was not called");

    // 割り当てを変えたあとの RemoveParticipant が遅れて LiveKit に届いても（backend が打ち切ったあとに LiveKit が
    // 処理した場合を含む）、外れるのは前の割り当ての identity だけで、入り直した人の新しい identity は外れない。
    // このルームへの RemoveParticipant は、接続中の identity を持つ mock が受ける。応答は 60 秒保留し、
    // backend の LiveKit の呼び出しは 10 秒で打ち切る。要求の反映（identity を外す）は、打ち切りのあとに
    // テストが `apply_late` で行う（LiveKit が古い要求を遅れて処理した状況）。
    let lk_room2 = format!("call-{channel}-{epoch_mesh}");
    let (status, joined) = request(
        &app,
        &users[3],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[3]})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "livekit");
    let first_identity = token_sub(joined["token"].as_str().unwrap());
    let room_state = LiveKitRoomState::default();
    room_state.connect(&first_identity);
    Mock::given(method("POST"))
        .and(path("/twirp/livekit.RoomService/RemoveParticipant"))
        .and(body_partial_json(json!({"room": lk_room2})))
        .respond_with(LateRemove {
            state: room_state.clone(),
            delay: Duration::from_secs(60),
        })
        .with_priority(1)
        .mount(&lk)
        .await;
    let lk_before = request_count(&lk).await;
    // fallback で Cloudflare に移る（前の LiveKit の割り当ての後始末が記録される）。
    let (status, joined) = request(
        &app,
        &users[3],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[3], "fallback": true})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "cloudflare");
    sfu.departures_wake.notify_one();
    let mut removing = false;
    for _ in 0..100 {
        removing = remove_participant_calls(&lk, lk_before, &lk_room2, &first_identity).await > 0;
        if removing {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(removing, "the worker did not remove the old identity");
    let removal_sent = tokio::time::Instant::now();
    // 応答を保留したまま（ロックはないので待たされない）、同じルームに入り直す。
    let (status, joined) = request(
        &app,
        &users[3],
        "POST",
        &join_uri,
        Some(json!({"session_id": sessions[3]})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "livekit");
    let second_identity = token_sub(joined["token"].as_str().unwrap());
    assert_ne!(second_identity, first_identity);
    assert!(second_identity.starts_with(&format!("{}.", sessions[3])));
    // 新しいトークンで LiveKit につなぐ。前の identity の削除は、まだ反映されていない。
    room_state.connect(&second_identity);
    assert!(room_state.is_connected(&first_identity));
    // join が終わった時点で、前の identity の RemoveParticipant はまだ応答待ち（打ち切りの前）。
    let departure_state = format!(
        "SELECT attempts::text || ':' || (claimed_until > statement_timestamp() + interval '60 seconds')::text
         FROM call_sfu_departures WHERE channel_id = $1 AND livekit_identity = '{first_identity}'"
    );
    assert!(removal_sent.elapsed() < Duration::from_secs(9));
    assert_eq!(
        text(&mut db, &departure_state, &channel).await?,
        "0:true",
        "the removal was no longer in flight when join returned"
    );
    // backend が打ち切るまで待つ（attempts が増え、再試行待ちになる）。
    let mut timed_out = false;
    for _ in 0..60 {
        if text(&mut db, &departure_state, &channel).await? == "1:false" {
            timed_out = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(timed_out, "the removal did not time out");
    // 打ち切られたあとに、LiveKit が受けていた要求を処理する。
    room_state.apply_late();
    assert!(
        !room_state.is_connected(&first_identity),
        "the late removal was not applied to the old identity"
    );
    assert!(
        room_state.is_connected(&second_identity),
        "the late removal disconnected the new identity"
    );
    // 打ち切られた要求（LiveKit があとで処理しうるもの）も、ほかの要求も、前の identity だけを指す。
    assert!(remove_participant_calls(&lk, lk_before, &lk_room2, &first_identity).await >= 1);
    assert_eq!(
        remove_participant_calls(&lk, lk_before, &lk_room2, &second_identity).await,
        0,
        "a removal targeted the new identity"
    );
    // 新しい割り当ては、新しい identity のまま。
    assert_eq!(
        text(
            &mut db,
            &format!(
                "SELECT livekit_identity FROM call_presence WHERE channel_id = $1 AND session_id = '{}'",
                sessions[3]
            ),
            &channel
        )
        .await?,
        second_identity
    );
    // 再試行を止める（mock は応答しないので、残すとこのあとの検査に入る）。
    toasty::sql::statement(
        "DELETE FROM call_sfu_departures WHERE channel_id = $1 AND livekit_identity = $2",
    )
    .bind(channel.as_str())
    .bind(first_identity.as_str())
    .exec(&mut db)
    .await?;

    // Cloudflare の close が応答待ちのままでも、LiveKit の後始末は待たない（SFU ごとに確保と処理を分ける）。
    // ほかの検査の件数に入らないよう、別のチャンネル ID で記録する（call_sfu_departures に外部キーはない）。
    let hang_channel = format!("hang-{}", uuid::Uuid::new_v4());
    let hang_cf = format!("cf-hang-{}", uuid::Uuid::new_v4());
    // mock は 60 秒たつまで応答しない。backend の Cloudflare の呼び出しは 10 秒で打ち切るので、
    // 行を記録してから 10 秒のあいだは、その呼び出しは必ず処理中のまま。
    Mock::given(method("GET"))
        .and(path(format!("/apps/app/sessions/{hang_cf}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"tracks": []}))
                .set_delay(Duration::from_secs(60)),
        )
        .with_priority(1)
        .mount(&cf)
        .await;
    let cf_recorded_at = tokio::time::Instant::now();
    toasty::sql::statement(
        "INSERT INTO call_sfu_departures (channel_id, session_id, sfu_id, cf_session_id)
         VALUES ($1, $2::uuid, 'cloudflare', $3)",
    )
    .bind(hang_channel.as_str())
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(hang_cf.as_str())
    .exec(&mut db)
    .await?;
    sfu.departures_wake.notify_one();
    // Cloudflare の行が確保され、応答待ちになるまで待つ。
    let mut hanging = false;
    for _ in 0..40 {
        hanging = cf
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|r| r.url.path() == format!("/apps/app/sessions/{hang_cf}"));
        if hanging {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(hanging, "the Cloudflare departure was not claimed");
    // そのあとに記録した LiveKit の行は、Cloudflare の応答（最長 10 秒で打ち切り）を待たずに処理される。
    let hang_room = format!("call-{hang_channel}-1");
    let hang_identity = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO call_sfu_departures (channel_id, session_id, sfu_id, sfu_room)
         VALUES ($1, $2::uuid, 'livekit', $3)",
    )
    .bind(hang_channel.as_str())
    .bind(hang_identity.as_str())
    .bind(hang_room.as_str())
    .exec(&mut db)
    .await?;
    sfu.departures_wake.notify_one();
    // RemoveParticipant は、Cloudflare の呼び出しが打ち切られうる時刻（記録から 10 秒）より前に届く。
    let cf_deadline = cf_recorded_at + Duration::from_secs(9);
    let mut removed_while_hanging = false;
    while tokio::time::Instant::now() < cf_deadline {
        if remove_participant_calls(&lk, 0, &hang_room, &hang_identity).await > 0 {
            removed_while_hanging = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        removed_while_hanging,
        "a LiveKit departure waited for a hanging Cloudflare close"
    );
    // Cloudflare の行は、まだ処理中（確保した直後のまま）。失敗して再試行を待つ行なら、attempts が増え、
    // claimed_until は 15 秒先になる。確保中なら attempts は 0 で、claimed_until は 5 分先。
    assert_eq!(
        text(
            &mut db,
            "SELECT attempts::text || ':' || (claimed_until > statement_timestamp() + interval '60 seconds')::text
             FROM call_sfu_departures WHERE channel_id = $1 AND sfu_id = 'cloudflare'",
            &hang_channel
        )
        .await?,
        "0:true",
        "the Cloudflare close was no longer in flight"
    );
    assert!(tokio::time::Instant::now() < cf_recorded_at + Duration::from_secs(10));
    toasty::sql::statement("DELETE FROM call_sfu_departures WHERE channel_id = $1")
        .bind(hang_channel.as_str())
        .exec(&mut db)
        .await?;

    // 全員が退出すると、次の入室で mesh に戻り、epoch が進む。
    drop(streams);
    for s in &sessions[..5] {
        toasty::sql::statement(
            "DELETE FROM call_presence WHERE channel_id = $1 AND session_id = $2::uuid",
        )
        .bind(channel.as_str())
        .bind(s.to_string())
        .exec(&mut db)
        .await?;
    }
    let (status, room, _stream) =
        open_stream(&app, &users[5], &channel, uuid::Uuid::new_v4(), false).await?;
    assert_eq!(status, StatusCode::OK);
    let room = room.expect("room_state");
    assert_eq!(room["mode"], "mesh");
    assert_eq!(room["epoch"].as_i64(), Some(epoch_mesh + 1));
    assert_eq!(room["max_participants"], 6);

    // bridge がないとき：Cloudflare のセッションを作るあいだ（ロックを外している）に LAN の人が LiveKit に
    // 入っても、取り直したロックの下で数え直し、LiveKit の人に sfu_reassign を送る。
    // 同じ registry と DB で、bridge を無効にし、Cloudflare の sessions/new を遅らせた SFU を使う。
    let slow_cf = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/apps/app/sessions/new$"))
        .respond_with(SlowNewSession(Duration::from_millis(1500)))
        .mount(&slow_cf)
        .await;
    let no_bridge = json!({
        "cloudflare": {"api_base": slow_cf.uri()},
        "livekit": {
            "ws_url": "wss://livekit.example",
            "api_url": lk.uri(),
            "lan_egress_cidrs": [format!("{LAN_IP}/32")]
        }
    })
    .to_string();
    let no_bridge_cfg = backend::call_sfu::CallSfuConfig::from_lookup(&|name| match name {
        "CALL_SFUS_JSON" => Some(no_bridge.clone()),
        "CF_REALTIME_APP_ID" => Some("app".into()),
        "CF_REALTIME_APP_SECRET" => Some("secret".into()),
        "LIVEKIT_API_KEY" => Some("key".into()),
        "LIVEKIT_API_SECRET" => Some("0123456789abcdef0123456789abcdef".into()),
        _ => None,
    })
    .map_err(anyhow::Error::msg)?
    .expect("sfu config");
    let mut state2 = state.clone();
    state2.call_sfu = Some(backend::call_sfu::CallSfu::new(no_bridge_cfg));
    assert!(!state2.call_sfu.as_ref().unwrap().bridge_enabled());
    let app2 = create_router(state2);
    let channel2 = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
    )
    .bind(channel2.as_str())
    .bind(users[0].as_str())
    .exec(&mut db)
    .await?;
    let sessions2: Vec<uuid::Uuid> = (0..5).map(|_| uuid::Uuid::new_v4()).collect();
    let mut streams2 = Vec::new();
    for i in 0..5 {
        let (status, _, stream) =
            open_stream(&app2, &users[i], &channel2, sessions2[i], true).await?;
        assert_eq!(status, StatusCode::OK, "join {i}");
        streams2.push(stream.expect("stream"));
    }
    let join2_uri = format!("/api/channels/{channel2}/call/sfu/join");
    // Cloudflare の人の join（sessions/new が遅い）。
    let cf_join = {
        let app = app2.clone();
        let user = users[1].clone();
        let uri = join2_uri.clone();
        let body = json!({"session_id": sessions2[1]});
        tokio::spawn(async move { request(&app, &user, "POST", &uri, Some(body), None).await })
    };
    let mut creating = false;
    for _ in 0..40 {
        creating = !slow_cf
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty();
        if creating {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(creating, "the Cloudflare join did not reach sessions/new");
    // そのあいだに LAN の人が入ると、部屋にはまだ Cloudflare の人がいないので LiveKit になる。
    let (status, joined) = request(
        &app2,
        &users[0],
        "POST",
        &join2_uri,
        Some(json!({"session_id": sessions2[0]})),
        Some(LAN_IP),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "livekit");
    assert!(
        !cf_join.is_finished(),
        "the Cloudflare join was not pending"
    );
    let (status, joined) = cf_join.await??;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["sfu"], "cloudflare");
    // LiveKit の人の SSE に sfu_reassign が届く。
    let lk_stream = &mut streams2[0];
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let reassigned = loop {
        if buf
            .split("\n\n")
            .any(|ev| ev.lines().any(|l| l.trim() == "event: sfu_reassign"))
        {
            break true;
        }
        match tokio::time::timeout_at(deadline, lk_stream.next()).await {
            Ok(Some(chunk)) => buf.push_str(&String::from_utf8(chunk?.to_vec())?),
            _ => break false,
        }
    };
    assert!(
        reassigned,
        "the LiveKit participant did not get sfu_reassign: {buf}"
    );
    drop(streams2);
    toasty::sql::statement("DELETE FROM call_presence WHERE channel_id = $1")
        .bind(channel2.as_str())
        .exec(&mut db)
        .await?;

    Ok(())
}

/// `join_latency` の 1 回分の並べ方。
#[derive(Clone, Copy, Debug)]
enum JoinMode {
    /// 同じ部屋の N 人が同時に join する（チャンネル行のロックを取り合う）。
    SameRoomConcurrent,
    /// 同じ部屋の N 人が 1 人ずつ join する（待ちなし）。
    SameRoomSequential,
    /// N 個の別々の部屋で 1 人ずつ、同時に join する（ロックは取り合わず、接続のプールなどは共有する）。
    SeparateRoomsConcurrent,
}

/// 部屋を用意して join を出し、要求ごとの時間を返す。部屋は SFU にするため 5 人で入る。
async fn join_round(
    app: &Router,
    db: &mut toasty::Db,
    n: usize,
    mode: JoinMode,
    timings_key: &str,
) -> anyhow::Result<Vec<(Duration, backend::routes::call_sfu::JoinTimings)>> {
    let rooms = match mode {
        JoinMode::SeparateRoomsConcurrent => n,
        _ => 1,
    };
    let per_room = match mode {
        JoinMode::SeparateRoomsConcurrent => 5,
        _ => n,
    };
    // (チャンネル, ユーザー, セッション) の、join を出す人。
    let mut joiners = Vec::new();
    let mut streams = Vec::new();
    let mut channels = Vec::new();
    for _ in 0..rooms {
        let channel = uuid::Uuid::new_v4().to_string();
        let users: Vec<String> = (0..per_room)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        for (i, id) in users.iter().enumerate() {
            toasty::sql::statement(
                "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
            )
            .bind(id.as_str())
            .bind(format!("Bench{i}"))
            .exec(&mut *db)
            .await?;
        }
        toasty::sql::statement(
            "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
        )
        .bind(channel.as_str())
        .bind(users[0].as_str())
        .exec(&mut *db)
        .await?;
        let sessions: Vec<uuid::Uuid> = (0..per_room).map(|_| uuid::Uuid::new_v4()).collect();
        for i in 0..per_room {
            let (status, _, stream) =
                open_stream(app, &users[i], &channel, sessions[i], true).await?;
            assert_eq!(status, StatusCode::OK, "stream {i}");
            streams.push(stream);
        }
        let join_count = if rooms == 1 { per_room } else { 1 };
        for i in 0..join_count {
            joiners.push((channel.clone(), users[i].clone(), sessions[i]));
        }
        channels.push(channel);
    }
    let join_one = |k: usize| {
        let app = app.clone();
        let (channel, user, session) = joiners[k].clone();
        // 半分は LAN（LiveKit）、半分は Cloudflare。
        let ip = if k.is_multiple_of(2) {
            Some(LAN_IP)
        } else {
            None
        };
        // 要求ごとに記録先を分け、join 全体の時間と段階ごとの時間を組にする。
        let key = format!("{timings_key}-{}", uuid::Uuid::new_v4());
        async move {
            let uri = format!("/api/channels/{channel}/call/sfu/join");
            // 送った時刻。backend は handler の始まりとの差（AuthUser の取り出しなど）を残す。
            let sent_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_micros()
                .to_string();
            let headers = vec![
                ("X-Test-Join-Timings".to_string(), key.clone()),
                ("X-Test-Join-Sent-At".to_string(), sent_at),
            ];
            let started = std::time::Instant::now();
            let (status, body) = request_with(
                &app,
                &user,
                "POST",
                &uri,
                Some(json!({"session_id": session})),
                ip,
                &headers,
            )
            .await?;
            let elapsed = started.elapsed();
            anyhow::ensure!(status == StatusCode::OK, "{status} {body}");
            Ok::<_, anyhow::Error>((
                elapsed,
                backend::routes::call_sfu::take_join_timings_for_test(&key),
            ))
        }
    };
    let mut lat = Vec::new();
    match mode {
        JoinMode::SameRoomSequential => {
            for k in 0..joiners.len() {
                lat.push(join_one(k).await?);
            }
        }
        _ => {
            let tasks: Vec<_> = (0..joiners.len())
                .map(|k| tokio::spawn(join_one(k)))
                .collect();
            for t in tasks {
                lat.push(t.await??);
            }
        }
    }
    drop(streams);
    for channel in &channels {
        toasty::sql::statement("DELETE FROM call_presence WHERE channel_id = $1")
            .bind(channel.as_str())
            .exec(&mut *db)
            .await?;
    }
    Ok(lat)
}

/// 要求ごとの記録をまとめたもの。
#[derive(Default)]
struct JoinSummary {
    join: Vec<Duration>,
    /// 要求ごとの、接続の取得の合計。
    db_acquire_total: Vec<Duration>,
    /// 要求ごとの、SQL の合計（BEGIN・ロック・COMMIT・ロックのあとの `SELECT 1` を含む）。
    db_sql_total: Vec<Duration>,
    /// 送ってから handler が始まるまで（AuthUser の取り出しの DB を含む）。
    pre_handler: Vec<Duration>,
    /// join − db_acquire_total − db_sql_total − pre_handler（Cloudflare の API、トークンの署名、応答の組み立てなど）。
    other: Vec<Duration>,
    /// 段階ごと（`acquire:名前`・`sql:名前`）の時間。
    phases: std::collections::BTreeMap<String, Vec<Duration>>,
    lock_wait: Vec<Duration>,
}

fn merge_join_timings(
    rows: Vec<(Duration, backend::routes::call_sfu::JoinTimings)>,
) -> JoinSummary {
    let mut sum = JoinSummary::default();
    for (elapsed, t) in rows {
        let acquire: Duration = t.acquire.values().flatten().sum();
        let sql: Duration = t.sql.values().flatten().sum();
        let pre: Duration = t.pre_handler.iter().sum();
        sum.join.push(elapsed);
        sum.db_acquire_total.push(acquire);
        sum.db_sql_total.push(sql);
        sum.pre_handler.extend(t.pre_handler);
        sum.other.push(elapsed.saturating_sub(acquire + sql + pre));
        for (name, v) in t.acquire {
            sum.phases
                .entry(format!("acquire:{name}"))
                .or_default()
                .extend(v);
        }
        for (name, v) in t.sql {
            sum.phases
                .entry(format!("sql:{name}"))
                .or_default()
                .extend(v);
        }
        sum.lock_wait.extend(t.lock_wait);
    }
    sum
}

/// `sfu/join` の待ち時間の計測（手で実行する。`cargo test --test call_sfu_test -- --ignored join_latency --nocapture`）。
///
/// 1 つの SFU の部屋に N 人を入れ、全員の `sfu/join` を同時に出したとき・1 人ずつ出したとき、N 個の別々の
/// 部屋で同時に出したときの、要求ごとの時間を比べる。join はチャンネル行を FOR UPDATE でロックするので、
/// 「同じ部屋で同時」と「別々の部屋で同時」の差が、チャンネル行のロック待ち。半分は LAN（LiveKit）、
/// 半分は Cloudflare（セッションを作る mock の遅延 0 / 100 ミリ秒）。DB は共有の DB をそのまま使う。
/// 各条件を 3 回測る（20 人で 60、50 人で 150 サンプル）。
///
/// cold は、遅延の条件ごとに新しく作った接続のプール（`init_db`）と AppState で、最初に出す 30 人の join
/// （同じ部屋で同時）。部屋の用意（SSE での入室）は同じプールで先に済ませるので、プールには接続があるが、
/// join の SQL はその接続で初めて実行する。
///
/// join の時間とは別に、要求ごとの記録先（`X-Test-Join-Timings`。ほかのテストの要求は混ざらない）に、
/// 段階ごとの時間を出す。join の中のすべての DB の段階を、接続の取得（`acquire:名前`）と SQL（`sql:名前`。BEGIN・
/// チャンネル行の FOR UPDATE・COMMIT・ロックのあとの `SELECT 1`（`_rtt`）を含む）に分けて測る。名前の `2` は、
/// Cloudflare の join が API を呼んだあとに取り直すトランザクション。要求ごとに、db_acquire_total（取得の合計）、
/// db_sql_total（SQL の合計）、pre_handler（送ってから handler が始まるまで。AuthUser の取り出しの DB を含む）、
/// other（join から、その 3 つを引いたもの。Cloudflare の API、トークンの署名、応答など）を出す。lock_wait は
/// ロックの文から `_rtt` を引いた、ロック待ちの目安。`JOIN_LATENCY_WARMUP=1` なら、測る前に共有のプールで
/// 20 人の同時 join を 1 回捨てる。
#[tokio::test]
#[ignore]
async fn join_latency() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let base_state = AppState::new(db.clone(), config.clone());
    for _ in 0..50 {
        if base_state.call_registry.is_listening() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    base_state.call_registry.set_db(db.clone()).await;
    let lk = MockServer::start().await;
    let warmup = std::env::var("JOIN_LATENCY_WARMUP").is_ok_and(|v| v == "1");

    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let summary = |all: &mut Vec<Duration>| {
        all.sort();
        let pick = |q: f64| all[((all.len() as f64 * q).ceil() as usize).clamp(1, all.len()) - 1];
        format!(
            "samples={} p50={:.1}ms p95={:.1}ms max={:.1}ms",
            all.len(),
            ms(pick(0.5)),
            ms(pick(0.95)),
            ms(*all.last().unwrap()),
        )
    };
    let mut report = Vec::new();
    for delay_ms in [0u64, 100] {
        let cf = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/apps/app/sessions/new$"))
            .respond_with(SlowNewSession(Duration::from_millis(delay_ms)))
            .mount(&cf)
            .await;
        let json = json!({
            "cloudflare": {"api_base": cf.uri()},
            "livekit": {
                "ws_url": "wss://livekit.example",
                "api_url": lk.uri(),
                "lan_egress_cidrs": [format!("{LAN_IP}/32")]
            },
            "bridge": true
        })
        .to_string();
        let cfg = backend::call_sfu::CallSfuConfig::from_lookup(&|name| match name {
            "CALL_SFUS_JSON" => Some(json.clone()),
            "CF_REALTIME_APP_ID" => Some("app".into()),
            "CF_REALTIME_APP_SECRET" => Some("secret".into()),
            "LIVEKIT_API_KEY" => Some("key".into()),
            "LIVEKIT_API_SECRET" => Some("0123456789abcdef0123456789abcdef".into()),
            "SFU_BRIDGE_TOKEN" => Some(BRIDGE_TOKEN.into()),
            _ => None,
        })
        .map_err(anyhow::Error::msg)?
        .expect("sfu config");
        let sfu = backend::call_sfu::CallSfu::new(cfg);
        let line =
            |label: String, rows: Vec<(Duration, backend::routes::call_sfu::JoinTimings)>| {
                let mut m = merge_join_timings(rows);
                let mut out = format!(
                    "{label}\n  join                 {}\n  db_acquire_total     {}\n  db_sql_total         {}\n  pre_handler          {}\n  other                {}\n  lock_wait            {}",
                    summary(&mut m.join),
                    summary(&mut m.db_acquire_total),
                    summary(&mut m.db_sql_total),
                    summary(&mut m.pre_handler),
                    summary(&mut m.other),
                    summary(&mut m.lock_wait),
                );
                for (name, v) in m.phases.iter_mut() {
                    out.push_str(&format!("\n  {name:<20} {}", summary(v)));
                }
                out
            };

        // cold：新しいプールと AppState で、最初の join。
        {
            let mut cold_db = init_db(&config.database_url).await?;
            let mut cold_state = AppState::new(cold_db.clone(), config.clone());
            cold_state.call_registry.set_db(cold_db.clone()).await;
            cold_state.call_sfu = Some(sfu.clone());
            let cold_app = create_router(cold_state);
            let rows = join_round(
                &cold_app,
                &mut cold_db,
                30,
                JoinMode::SameRoomConcurrent,
                "cold",
            )
            .await?;
            report.push(line(
                format!("cf_delay={delay_ms}ms n=30 cold SameRoomConcurrent"),
                rows,
            ));
        }

        let mut state = base_state.clone();
        state.call_sfu = Some(sfu);
        let app = create_router(state);
        // JOIN_LATENCY_WARMUP=1 なら、測る前に 20 人の同時 join を 1 回捨てる（プールの接続を作っておく）。
        if warmup && delay_ms == 0 {
            join_round(&app, &mut db, 20, JoinMode::SameRoomConcurrent, "warmup").await?;
            report.push(
                "(warm-up: one discarded round of 20 concurrent joins on the shared pool)".into(),
            );
        }
        for n in [20usize, 50] {
            for mode in [
                JoinMode::SameRoomConcurrent,
                JoinMode::SeparateRoomsConcurrent,
                JoinMode::SameRoomSequential,
            ] {
                let mut rows = Vec::new();
                for _ in 0..3 {
                    rows.extend(join_round(&app, &mut db, n, mode, "warm").await?);
                }
                report.push(line(format!("cf_delay={delay_ms}ms n={n} {mode:?}"), rows));
            }
        }
    }
    for line in &report {
        println!("{line}");
    }
    Ok(())
}
