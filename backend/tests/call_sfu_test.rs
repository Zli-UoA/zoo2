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
    matchers::{method, path, path_regex},
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
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Dev-User", actor)
        .header("content-type", "application/json")
        .body(match body {
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
    Ok(s.parse()?)
}

#[tokio::test]
async fn sfu_mode_switch_assignment_and_cleanup() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let cf = MockServer::start().await;
    let lk = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/apps/app/sessions/new$"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sessionId": format!("cf-{}", uuid::Uuid::new_v4())
        })))
        .mount(&cf)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/apps/app/sessions/[^/]+/tracks/new$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sessionDescription": {"type": "answer", "sdp": "v=0\r\n"},
            "tracks": [{"mid": "0", "trackName": "t"}],
            "requiresImmediateRenegotiation": false
        })))
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tracks": []})))
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
        std::env::set_var("CALL_SFU_THRESHOLD", "4");
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
    // 同じ revision なら待つ（ここでは変化が来るまで待たずに期限で返る場合もある）。
    let revision = rooms["revision"].as_str().unwrap().to_string();
    let polled = tokio::time::timeout(
        Duration::from_secs(30),
        bridge(
            "GET",
            &format!("/internal/sfu-bridge/rooms?bridge_epoch={bridge_epoch}&since={revision}"),
            None,
        ),
    )
    .await?;
    assert_eq!(polled.0, StatusCode::OK);

    // 新しい bridge が登録すると、前の bridge のトラックは消える。
    let (_, _) = bridge("POST", "/internal/sfu-bridge/register", None).await;
    assert_eq!(
        count(
            &mut db,
            "SELECT count(*) FROM call_sfu_tracks WHERE channel_id = $1 AND source = 'bridge'",
            &channel
        )
        .await?,
        0
    );

    // Cloudflare の人が退出すると、トラックが消え、worker が Cloudflare のセッションを閉じる。
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
    let closes = cf
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/tracks/close"))
        .count();
    assert!(closes >= 1);

    // LiveKit の人が退出すると、RemoveParticipant を呼ぶ。
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
    let mut removed = false;
    for _ in 0..60 {
        if !lk.received_requests().await.unwrap_or_default().is_empty() {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(removed, "LiveKit RemoveParticipant was not called");

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

    Ok(())
}
