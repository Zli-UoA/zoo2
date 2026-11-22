//! 通話の SFU の API（docs/calls-sfu.md）。
//!
//! - `POST /api/channels/{id}/call/sfu/join`：SFU に割り当てる（LiveKit のトークン、または Cloudflare のセッション）。
//! - `POST /api/channels/{id}/call/sfu/cf/{publish,pull,answer}`：Cloudflare Realtime の代理。
//!   pull する相手は backend が一覧から決め、クライアントに Cloudflare のセッション ID や track 名を指定させない。
//! - `/internal/sfu-bridge/...`：sfu-bridge 用（`SFU_BRIDGE_TOKEN`）。
//!
//! SFU が無効（`CALL_SFUS_JSON` が未設定）なら、どれも 404。

use std::{net::SocketAddr, time::Duration};

use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{Extensions, HeaderMap, header},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    call_sfu::{
        CallSfu, RoomMode, SFU_CLOUDFLARE, SFU_LIVEKIT,
        cloudflare::{RemoteTrack, SessionDescription, TracksResponse},
        livekit, rows_json,
    },
    error::AppError,
    routes::calls::{
        CallDistributedEvent, SfuTrackInfo, authorize_call_access, load_sfu_tracks,
    },
    state::AppState,
};

/// 1 回の pull で指定できるトラックの上限。
const MAX_PULL_TRACKS: usize = 64;
/// sfu-bridge の long-poll の待ち時間。
const BRIDGE_POLL_WAIT: Duration = Duration::from_secs(25);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{id}/call/sfu/join", post(join))
        .route("/api/channels/{id}/call/sfu/cf/publish", post(cf_publish))
        .route("/api/channels/{id}/call/sfu/cf/pull", post(cf_pull))
        .route("/api/channels/{id}/call/sfu/cf/answer", post(cf_answer))
        .route("/internal/sfu-bridge/register", post(bridge_register))
        .route("/internal/sfu-bridge/rooms", get(bridge_rooms))
        .route("/internal/sfu-bridge/cf/session", post(bridge_cf_session))
        .route("/internal/sfu-bridge/cf/publish", post(bridge_cf_publish))
        .route("/internal/sfu-bridge/cf/pull", post(bridge_cf_pull))
        .route("/internal/sfu-bridge/cf/answer", post(bridge_cf_answer))
        .route("/internal/sfu-bridge/cf/close", post(bridge_cf_close))
        // SDP が入るだけの小さな本文。
        .layer(DefaultBodyLimit::max(64 * 1024))
}

fn sfu(state: &AppState) -> Result<&CallSfu, AppError> {
    state
        .call_sfu
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Call SFU is not enabled".into()))
}

/// 在室中の自分のセッション。
#[derive(Debug, Deserialize)]
struct PresenceRow {
    display_name: String,
    sfu_id: Option<String>,
    sfu_room: Option<String>,
    cf_session_id: Option<String>,
    mode: Option<RoomMode>,
    epoch: Option<i64>,
}

/// セッションが認証ユーザーのもので、まだ在室しているかを確かめる。探索は call_presence の主キーで 1 行。
async fn own_presence(
    executor: &mut dyn toasty::Executor,
    channel_id: &str,
    session_id: Uuid,
    user_id: &str,
    lock: bool,
) -> Result<PresenceRow, AppError> {
    let lock = if lock { "FOR UPDATE OF p" } else { "" };
    let rows = toasty::sql::query(format!(
        "SELECT json_build_object(
             'display_name', p.display_name, 'sfu_id', p.sfu_id, 'sfu_room', p.sfu_room,
             'cf_session_id', p.cf_session_id, 'mode', r.mode, 'epoch', r.epoch)::text
         FROM call_presence p
         LEFT JOIN call_rooms r ON r.channel_id = p.channel_id
         WHERE p.channel_id = $1 AND p.session_id = $2::uuid AND p.user_id = $3
           AND p.expires_at > statement_timestamp()
         {lock}"
    ))
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .bind(user_id.to_string())
    .exec(executor)
    .await?;
    rows_json::<PresenceRow>(rows)
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("Call session not found".into()))
}

/// トラック一覧の版を進め、全レプリカに知らせる（bridge も起きる）。
async fn bump_tracks(executor: &mut dyn toasty::Executor, channel_id: &str) -> Result<(), AppError> {
    toasty::sql::query(
        "WITH bumped AS (
             UPDATE call_rooms SET track_version = track_version + 1
             WHERE channel_id = $1
             RETURNING channel_id, epoch, track_version
         )
         SELECT pg_notify('zoo_call_events', json_build_object(
             'type', 'sfu_tracks_changed', 'channel_id', channel_id,
             'epoch', epoch, 'version', track_version)::text)::text
         FROM bumped",
    )
    .bind(channel_id.to_string())
    .exec(executor)
    .await?;
    Ok(())
}

/// クライアントの IP。Cloudflare Tunnel 経由なら `CF-Connecting-IP`。わからなければ `None`
/// （そのときは Cloudflare に割り当てる）。
fn client_ip(extensions: &Extensions, headers: &HeaderMap) -> Option<std::net::IpAddr> {
    let peer = extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let tunnel = matches!(
        std::env::var("ACTIVITYPUB_CLOUDFLARE_TUNNEL").as_deref(),
        Ok("true" | "1")
    );
    crate::activitypub::client_ip::resolve(peer, headers, tunnel)
        .ok()
        .flatten()
}

#[derive(Debug, Deserialize)]
struct JoinBody {
    session_id: Uuid,
    /// LiveKit につながらなかったので、Cloudflare でやり直す。
    #[serde(default)]
    fallback: bool,
    /// LiveKit を使えないクライアント（Android の WebView は外への接続を許していない）。
    #[serde(default)]
    cloudflare_only: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "sfu", rename_all = "lowercase")]
enum JoinTarget {
    Livekit { url: String, token: String },
    Cloudflare,
}

#[derive(Debug, Serialize)]
struct JoinResponse {
    #[serde(flatten)]
    target: JoinTarget,
    epoch: i64,
    version: i64,
    tracks: Vec<SfuTrackInfo>,
}

async fn join(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(body): Json<JoinBody>,
) -> Result<Json<JoinResponse>, AppError> {
    let sfu = sfu(&state)?;
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;
    let lan_client = sfu.is_lan_client(client_ip(&extensions, &headers));

    // 割り当ては、チャンネル行のロックの下で決める（bridge がないときの「部屋に Cloudflare の人がいるか」を
    // 入室と直列化するため）。
    let mut tx = db.transaction().await?;
    toasty::sql::query("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel_id.clone())
        .exec(&mut tx)
        .await?;
    let me = own_presence(&mut tx, &channel_id, body.session_id, &user.id, true).await?;
    let (Some(RoomMode::Sfu), Some(epoch)) = (me.mode, me.epoch) else {
        return Err(AppError::Conflict("Call is not in SFU mode".into()));
    };
    let others = toasty::sql::query(
        "SELECT json_build_object(
             'cloudflare', count(*) FILTER (WHERE sfu_id = 'cloudflare'),
             'livekit', count(*) FILTER (WHERE sfu_id = 'livekit'))::text
         FROM call_presence
         WHERE channel_id = $1 AND session_id <> $2::uuid AND expires_at > statement_timestamp()",
    )
    .bind(channel_id.clone())
    .bind(body.session_id.to_string())
    .exec(&mut tx)
    .await?;
    #[derive(Deserialize, Default)]
    struct Counts {
        cloudflare: i64,
        livekit: i64,
    }
    let counts = rows_json::<Counts>(others).into_iter().next().unwrap_or_default();
    let choice = crate::call_sfu::choose_sfu(
        lan_client,
        sfu.livekit.is_some(),
        sfu.bridge_enabled(),
        body.fallback || body.cloudflare_only,
        counts.cloudflare > 0,
    );

    // 前の割り当て（fallback・やり直し）の後始末は、退出と同じ worker に任せる。
    let previous_cf = me.cf_session_id.clone();
    let previous_livekit_room = (me.sfu_id.as_deref() == Some(SFU_LIVEKIT))
        .then(|| me.sfu_room.clone())
        .flatten();

    let target = if choice == SFU_LIVEKIT {
        let lk = sfu.livekit.as_ref().expect("choose_sfu returns livekit only when enabled");
        let room = livekit::room_name(&channel_id, epoch);
        let token = lk.participant_token(&room, &body.session_id.to_string(), &me.display_name)?;
        set_assignment(&mut tx, &channel_id, body.session_id, SFU_LIVEKIT, Some(&room), None).await?;
        bump_tracks(&mut tx, &channel_id).await?;
        tx.commit().await?;
        JoinTarget::Livekit {
            url: lk.ws_url().to_string(),
            token,
        }
    } else {
        // Cloudflare の API は、ロックを持ったまま呼ばない。
        tx.commit().await?;
        let cf_session = sfu.cloudflare.new_session().await?;
        let mut tx = db.transaction().await?;
        // 呼んでいるあいだに退出していたら、作ったセッションは使わない（何も流れないので放置してよい）。
        own_presence(&mut tx, &channel_id, body.session_id, &user.id, true).await?;
        set_assignment(
            &mut tx,
            &channel_id,
            body.session_id,
            SFU_CLOUDFLARE,
            None,
            Some(&cf_session),
        )
        .await?;
        bump_tracks(&mut tx, &channel_id).await?;
        if !sfu.bridge_enabled() && counts.livekit > 0 {
            // bridge がないので、LiveKit の人にも Cloudflare に移ってもらう。
            let event = CallDistributedEvent::SfuReassign {
                channel_id: channel_id.clone(),
                epoch,
            };
            notify_in(&mut tx, &event).await?;
        }
        tx.commit().await?;
        JoinTarget::Cloudflare
    };

    if previous_cf.is_some() || previous_livekit_room.is_some() {
        let mut db = state.db.clone();
        toasty::sql::statement(
            "INSERT INTO call_sfu_departures (channel_id, session_id, sfu_id, sfu_room, cf_session_id)
             VALUES ($1, $2::uuid, $3, NULLIF($4, ''), NULLIF($5, ''))",
        )
        .bind(channel_id.clone())
        .bind(body.session_id.to_string())
        .bind(if previous_livekit_room.is_some() {
            SFU_LIVEKIT
        } else {
            SFU_CLOUDFLARE
        })
        .bind(previous_livekit_room.unwrap_or_default())
        .bind(previous_cf.unwrap_or_default())
        .exec(&mut db)
        .await?;
        sfu.departures_wake.notify_one();
    }

    let (epoch, version, tracks) = load_sfu_tracks(&mut db, &channel_id)
        .await?
        .unwrap_or((epoch, 0, Vec::new()));
    Ok(Json(JoinResponse {
        target,
        epoch,
        version,
        tracks,
    }))
}

async fn set_assignment(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    session_id: Uuid,
    sfu_id: &str,
    room: Option<&str>,
    cf_session: Option<&str>,
) -> Result<(), AppError> {
    // 割り当てが変わったら、前の SFU に出していたトラックは消す。
    toasty::sql::statement(
        "DELETE FROM call_sfu_tracks WHERE channel_id = $1 AND session_id = $2::uuid",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .exec(&mut *tx)
    .await?;
    toasty::sql::statement(
        "UPDATE call_presence SET sfu_id = $3, sfu_room = NULLIF($4, ''), cf_session_id = NULLIF($5, '')
         WHERE channel_id = $1 AND session_id = $2::uuid",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .bind(sfu_id.to_string())
    // toasty は NULL の型を推定できないので、空文字を NULL として渡す。
    .bind(room.unwrap_or_default().to_string())
    .bind(cf_session.unwrap_or_default().to_string())
    .exec(&mut *tx)
    .await?;
    Ok(())
}

async fn notify_in(
    executor: &mut dyn toasty::Executor,
    event: &CallDistributedEvent,
) -> Result<(), AppError> {
    let payload = serde_json::to_string(event).map_err(|e| AppError::Internal(e.into()))?;
    toasty::sql::statement("SELECT pg_notify('zoo_call_events', $1)")
        .bind(payload)
        .exec(executor)
        .await?;
    Ok(())
}

/// Cloudflare に割り当てた自分のセッション。
async fn own_cf_session(
    state: &AppState,
    user: &AuthUser,
    channel_id: &str,
    session_id: Uuid,
) -> Result<String, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, channel_id, user).await?;
    let me = own_presence(&mut db, channel_id, session_id, &user.id, false).await?;
    match (me.sfu_id.as_deref(), me.cf_session_id) {
        (Some(SFU_CLOUDFLARE), Some(cf)) => Ok(cf),
        _ => Err(AppError::Conflict(
            "This session is not assigned to Cloudflare".into(),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct PublishBody {
    session_id: Uuid,
    offer: SessionDescription,
}

/// マイクの音声を Cloudflare に push する。SDP は音声 1 本だけ。
async fn cf_publish(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<PublishBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = sfu(&state)?;
    let cf = own_cf_session(&state, &user, &channel_id, body.session_id).await?;
    let mid = crate::call_sfu::cloudflare::validate_audio_only_offer(&body.offer)?;
    let track_name = body.session_id.to_string();
    let res = sfu
        .cloudflare
        .push_track(&cf, &body.offer, &mid, &track_name)
        .await?;
    register_track(&state, &channel_id, body.session_id, "direct", &cf, &track_name, None).await?;
    Ok(Json(res))
}

async fn register_track(
    state: &AppState,
    channel_id: &str,
    speaker: Uuid,
    source: &str,
    cf_session: &str,
    track_name: &str,
    bridge_epoch: Option<i64>,
) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 話す人がもう退出していれば、外部キーで失敗する前に何もしない。
    let inserted = toasty::sql::query(
        "INSERT INTO call_sfu_tracks (channel_id, session_id, source, cf_session_id, track_name, bridge_epoch)
         SELECT $1, $2::uuid, $3, $4, $5, NULLIF($6, '')::bigint
         WHERE EXISTS (SELECT 1 FROM call_presence WHERE channel_id = $1 AND session_id = $2::uuid)
         ON CONFLICT (channel_id, session_id, source) DO UPDATE
           SET cf_session_id = EXCLUDED.cf_session_id, track_name = EXCLUDED.track_name,
               bridge_epoch = EXCLUDED.bridge_epoch
         RETURNING session_id::text",
    )
    .bind(channel_id.to_string())
    .bind(speaker.to_string())
    .bind(source.to_string())
    .bind(cf_session.to_string())
    .bind(track_name.to_string())
    .bind(bridge_epoch.map(|e| e.to_string()).unwrap_or_default())
    .exec(&mut tx)
    .await?;
    if inserted.is_empty() {
        return Err(AppError::NotFound("Call session not found".into()));
    }
    bump_tracks(&mut tx, channel_id).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct PullBody {
    session_id: Uuid,
    track_ids: Vec<String>,
}

/// 同じ部屋のトラックだけを、Cloudflare のセッションと track 名に引き直す。`exclude` は自分の声。
async fn resolve_tracks(
    state: &AppState,
    channel_id: &str,
    track_ids: &[String],
    exclude: Option<Uuid>,
) -> Result<Vec<RemoteTrack>, AppError> {
    if track_ids.is_empty() || track_ids.len() > MAX_PULL_TRACKS {
        return Err(AppError::BadRequest(format!(
            "track_ids must have 1 to {MAX_PULL_TRACKS} items"
        )));
    }
    let mut keys = Vec::new();
    for id in track_ids {
        let (sid, source) = id
            .split_once(':')
            .ok_or_else(|| AppError::BadRequest("Invalid track id".into()))?;
        let sid: Uuid = sid
            .parse()
            .map_err(|_| AppError::BadRequest("Invalid track id".into()))?;
        if !matches!(source, "direct" | "bridge") {
            return Err(AppError::BadRequest("Invalid track id".into()));
        }
        if Some(sid) == exclude {
            continue;
        }
        keys.push(json!([sid, source]));
    }
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    // 探索：主キー (channel_id, session_id, source) で、指定した数（最大 MAX_PULL_TRACKS）だけ。
    let mut db = state.db.clone();
    let rows = toasty::sql::query(
        "SELECT json_build_object('session_id', t.cf_session_id, 'track_name', t.track_name)::text
         FROM jsonb_array_elements($2::jsonb) k
         JOIN call_sfu_tracks t
           ON t.channel_id = $1 AND t.session_id = (k->>0)::uuid AND t.source = k->>1",
    )
    .bind(channel_id.to_string())
    .bind(Value::Array(keys).to_string())
    .exec(&mut db)
    .await?;
    #[derive(Deserialize)]
    struct Row {
        session_id: String,
        track_name: String,
    }
    Ok(rows_json::<Row>(rows)
        .into_iter()
        .map(|r| RemoteTrack {
            session_id: r.session_id,
            track_name: r.track_name,
        })
        .collect())
}

/// ほかの人の声を pull する。応答に offer があれば、クライアントは `cf/answer` で answer を返す。
async fn cf_pull(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<PullBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = sfu(&state)?;
    let cf = own_cf_session(&state, &user, &channel_id, body.session_id).await?;
    let remotes = resolve_tracks(&state, &channel_id, &body.track_ids, Some(body.session_id)).await?;
    if remotes.is_empty() {
        return Err(AppError::NotFound("No tracks to pull".into()));
    }
    Ok(Json(sfu.cloudflare.pull_tracks(&cf, &remotes).await?))
}

#[derive(Debug, Deserialize)]
struct AnswerBody {
    session_id: Uuid,
    answer: SessionDescription,
}

async fn cf_answer(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<Value>, AppError> {
    let sfu = sfu(&state)?;
    if body.answer.kind != "answer" {
        return Err(AppError::BadRequest("sessionDescription must be an answer".into()));
    }
    let cf = own_cf_session(&state, &user, &channel_id, body.session_id).await?;
    sfu.cloudflare.renegotiate(&cf, &body.answer).await?;
    Ok(Json(json!({ "ok": true })))
}

// ---- sfu-bridge（内部 API） ----

fn bridge_auth<'a>(state: &'a AppState, headers: &HeaderMap) -> Result<&'a CallSfu, AppError> {
    let sfu = sfu(state)?;
    let Some(expected) = sfu.config.bridge_token.as_deref() else {
        return Err(AppError::NotFound("sfu-bridge is not enabled".into()));
    };
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let same = given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if same {
        Ok(sfu)
    } else {
        Err(AppError::Unauthorized("Invalid sfu-bridge token".into()))
    }
}

/// bridge の起動時に呼ぶ。新しい epoch を返し、前の bridge が出していたトラックを消す。
async fn bridge_register(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    bridge_auth(&state, &headers)?;
    let epoch = jiff::Timestamp::now().as_millisecond();
    let mut db = state.db.clone();
    // 探索：source = 'bridge' の行（bridge が中継中の話者の数）だけ。
    let rows = toasty::sql::query(
        "WITH gone AS (
             DELETE FROM call_sfu_tracks
             WHERE source = 'bridge' AND (bridge_epoch IS NULL OR bridge_epoch < $1::bigint)
             RETURNING channel_id
         ),
         bumped AS (
             UPDATE call_rooms r SET track_version = r.track_version + 1
             FROM (SELECT DISTINCT channel_id FROM gone) g
             WHERE r.channel_id = g.channel_id
             RETURNING r.channel_id, r.epoch, r.track_version
         )
         SELECT pg_notify('zoo_call_events', json_build_object(
             'type', 'sfu_tracks_changed', 'channel_id', channel_id,
             'epoch', epoch, 'version', track_version)::text)::text
         FROM bumped",
    )
    .bind(epoch.to_string())
    .exec(&mut db)
    .await?;
    let _ = rows;
    Ok(Json(json!({ "bridge_epoch": epoch })))
}

#[derive(Debug, Deserialize)]
struct RoomsQuery {
    bridge_epoch: i64,
    /// 前回の応答の `revision`。同じなら変化を待つ。
    #[serde(default)]
    since: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BridgeRoomState {
    channel_id: String,
    epoch: i64,
    /// LiveKit にいる人（この人たちの声を Cloudflare に push する）。
    livekit_sessions: Vec<Uuid>,
    /// Cloudflare の人の声（LiveKit に publish する）。
    cloudflare_tracks: Vec<SfuTrackInfo>,
}

#[derive(Debug, Serialize)]
struct BridgeRoom {
    #[serde(flatten)]
    state: BridgeRoomState,
    livekit_url: String,
    livekit_room: String,
    livekit_token: String,
}

/// LiveKit と Cloudflare の両方に人がいる部屋の一覧。変化がなければ最大 25 秒待つ。
///
/// 探索：LiveKit の在室者（部分インデックス idx_call_presence_livekit）の部屋ごとに、
/// その部屋の在室者とトラックを主キーの先頭で読む。在室者は部屋ごとに定員以下。
async fn bridge_rooms(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RoomsQuery>,
) -> Result<Json<Value>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    let lk = sfu
        .livekit
        .as_ref()
        .ok_or_else(|| AppError::NotFound("LiveKit is not enabled".into()))?;
    let deadline = tokio::time::Instant::now() + BRIDGE_POLL_WAIT;
    loop {
        // 読む前に待ち受けを作り、読んでいるあいだの通知を取りこぼさない。
        let notified = sfu.bridge_wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let rooms = load_bridge_rooms(&state).await?;
        let revision = revision_of(&rooms);
        let changed = query.since.as_deref() != Some(revision.as_str());
        if changed || tokio::time::Instant::now() >= deadline {
            let rooms = rooms
                .into_iter()
                .map(|r| {
                    let room = livekit::room_name(&r.channel_id, r.epoch);
                    let token = lk.bridge_token(&room, query.bridge_epoch)?;
                    Ok(BridgeRoom {
                        state: r,
                        livekit_url: lk.ws_url().to_string(),
                        livekit_room: room,
                        livekit_token: token,
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            return Ok(Json(json!({ "revision": revision, "rooms": rooms })));
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            // 期限まで変化なし。同じ revision で返す。
            continue;
        }
    }
}

fn revision_of(rooms: &[BridgeRoomState]) -> String {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(rooms).unwrap_or_default();
    let digest = Sha256::digest(&json);
    digest[..12].iter().map(|b| format!("{b:02x}")).collect()
}

async fn load_bridge_rooms(state: &AppState) -> Result<Vec<BridgeRoomState>, AppError> {
    let mut db = state.db.clone();
    let rows = toasty::sql::query(
        "WITH rooms AS (
             SELECT DISTINCT p.channel_id FROM call_presence p
             WHERE p.sfu_id = 'livekit' AND p.expires_at > statement_timestamp()
         )
         SELECT json_build_object(
             'channel_id', r.channel_id,
             'epoch', r.epoch,
             'livekit_sessions', (
                 SELECT json_agg(p.session_id ORDER BY p.session_id) FROM call_presence p
                 WHERE p.channel_id = r.channel_id AND p.sfu_id = 'livekit'
                   AND p.expires_at > statement_timestamp()),
             'cloudflare_tracks', COALESCE((
                 SELECT json_agg(json_build_object(
                     'id', t.session_id::text || ':' || t.source,
                     'session_id', t.session_id, 'source', t.source) ORDER BY t.session_id)
                 FROM call_sfu_tracks t
                 WHERE t.channel_id = r.channel_id AND t.source = 'direct'), '[]'::json)
         )::text
         FROM rooms
         JOIN call_rooms r ON r.channel_id = rooms.channel_id AND r.mode = 'sfu'
         WHERE EXISTS (
             SELECT 1 FROM call_presence c
             WHERE c.channel_id = r.channel_id AND c.sfu_id = 'cloudflare'
               AND c.expires_at > statement_timestamp())
         ORDER BY r.channel_id",
    )
    .exec(&mut db)
    .await?;
    Ok(rows_json::<BridgeRoomState>(rows))
}

#[derive(Debug, Deserialize)]
struct BridgeChannel {
    channel_id: String,
}

async fn bridge_cf_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BridgeChannel>,
) -> Result<Json<Value>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    let _ = body.channel_id;
    let session_id = sfu.cloudflare.new_session().await?;
    Ok(Json(json!({ "cf_session_id": session_id })))
}

#[derive(Debug, Deserialize)]
struct BridgePublishBody {
    channel_id: String,
    bridge_epoch: i64,
    cf_session_id: String,
    /// LiveKit にいる話者。
    speaker_session_id: Uuid,
    offer: SessionDescription,
    mid: String,
}

/// LiveKit の話者の声を、bridge の Cloudflare セッションから push する。
async fn bridge_cf_publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BridgePublishBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    crate::call_sfu::cloudflare::validate_audio_offer_mid(&body.offer, &body.mid)?;
    // 話者が本当にその部屋の LiveKit にいるか。
    let mut db = state.db.clone();
    let rows = toasty::sql::query(
        "SELECT 1 FROM call_presence
         WHERE channel_id = $1 AND session_id = $2::uuid AND sfu_id = 'livekit'
           AND expires_at > statement_timestamp()",
    )
    .bind(body.channel_id.clone())
    .bind(body.speaker_session_id.to_string())
    .exec(&mut db)
    .await?;
    if rows.is_empty() {
        return Err(AppError::NotFound("Speaker is not on LiveKit".into()));
    }
    let track_name = format!("{}-bridge", body.speaker_session_id);
    let res = sfu
        .cloudflare
        .push_track(&body.cf_session_id, &body.offer, &body.mid, &track_name)
        .await?;
    register_track(
        &state,
        &body.channel_id,
        body.speaker_session_id,
        "bridge",
        &body.cf_session_id,
        &track_name,
        Some(body.bridge_epoch),
    )
    .await?;
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
struct BridgePullBody {
    channel_id: String,
    cf_session_id: String,
    track_ids: Vec<String>,
}

/// Cloudflare の人の声を、bridge の Cloudflare セッションに pull する（direct のトラックだけ）。
async fn bridge_cf_pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BridgePullBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    if body.track_ids.iter().any(|id| !id.ends_with(":direct")) {
        return Err(AppError::BadRequest("bridge pulls direct tracks only".into()));
    }
    let remotes = resolve_tracks(&state, &body.channel_id, &body.track_ids, None).await?;
    if remotes.is_empty() {
        return Err(AppError::NotFound("No tracks to pull".into()));
    }
    Ok(Json(
        sfu.cloudflare
            .pull_tracks(&body.cf_session_id, &remotes)
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct BridgeAnswerBody {
    cf_session_id: String,
    answer: SessionDescription,
}

async fn bridge_cf_answer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BridgeAnswerBody>,
) -> Result<Json<Value>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    if body.answer.kind != "answer" {
        return Err(AppError::BadRequest("sessionDescription must be an answer".into()));
    }
    sfu.cloudflare
        .renegotiate(&body.cf_session_id, &body.answer)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
struct BridgeCloseBody {
    cf_session_id: String,
    mids: Vec<String>,
}

async fn bridge_cf_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BridgeCloseBody>,
) -> Result<Json<Value>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    if body.mids.len() > MAX_PULL_TRACKS {
        return Err(AppError::BadRequest("Too many mids".into()));
    }
    sfu.cloudflare
        .close_tracks(&body.cf_session_id, &body.mids)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

