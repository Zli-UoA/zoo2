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
        CallDistributedEvent, SfuTrackInfo, authorize_call_access, authorize_call_access_on,
        load_sfu_tracks,
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
    livekit_identity: Option<String>,
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
             'cf_session_id', p.cf_session_id, 'livekit_identity', p.livekit_identity,
             'mode', r.mode, 'epoch', r.epoch)::text
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
async fn bump_tracks(
    executor: &mut dyn toasty::Executor,
    channel_id: &str,
) -> Result<(), AppError> {
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
    // 計測（`X-Test-Join-Timings`。開発用の認証のときだけ）。`None` なら、どの段階も記録せず、処理も変わらない。
    let timings = JoinTimingKey::from_headers(&state, &headers);
    let t = timings.as_ref();
    if t.is_some() {
        // 計測：権限の確認を、取り出した 1 本の接続で行い、接続の取得と SQL を分けて測る。
        let mut conn = acquire(&db, t, "auth").await?;
        timed_sql(
            t,
            "auth",
            authorize_call_access_on(&mut conn, &channel_id, &user),
        )
        .await?;
    } else {
        authorize_call_access(&mut db, &channel_id, &user).await?;
    }
    let lan_client = sfu.is_lan_client(client_ip(&extensions, &headers));

    // 割り当ては、チャンネル行のロックの下で決める（bridge がないときの「部屋に Cloudflare の人がいるか」を
    // 入室と直列化するため）。
    // 接続の取得と BEGIN を分けて計測できるよう、接続を取ってからトランザクションを始める
    // （`Db::transaction` と同じ）。Cloudflare の API を呼ぶ前には、この接続をプールに返す。
    let mut conn = acquire(&db, t, "tx").await?;
    let mut tx = begin(&mut conn, t, "begin").await?;
    lock_channel(&mut tx, &channel_id, t, "lock").await?;
    let me = timed_sql(
        t,
        "own_presence",
        own_presence(&mut tx, &channel_id, body.session_id, &user.id, true),
    )
    .await?;
    let (Some(RoomMode::Sfu), Some(epoch)) = (me.mode, me.epoch) else {
        return Err(AppError::Conflict("Call is not in SFU mode".into()));
    };
    let counts = timed_sql(
        t,
        "count_others",
        count_others(&mut tx, &channel_id, body.session_id),
    )
    .await?;
    let choice = crate::call_sfu::choose_sfu(
        lan_client,
        sfu.livekit.is_some(),
        sfu.bridge_enabled(),
        body.fallback || body.cloudflare_only,
        counts.cloudflare > 0,
    );

    let (target, departed) = if choice == SFU_LIVEKIT {
        let lk = sfu
            .livekit
            .as_ref()
            .expect("choose_sfu returns livekit only when enabled");
        let room = livekit::room_name(&channel_id, epoch);
        // 同じルームへの割り当てを出し直すだけ（join の再送）なら、identity も同じにする（つないでいる接続は
        // そのまま）。それ以外は新しい identity にする（退出の後始末は、前の identity だけを外す）。
        let identity = match (&me.sfu_id, &me.sfu_room, &me.livekit_identity) {
            (Some(sfu_id), Some(r), Some(identity)) if sfu_id == SFU_LIVEKIT && *r == room => {
                identity.clone()
            }
            _ => livekit::participant_identity(&body.session_id.to_string()),
        };
        let token = lk.participant_token(&room, &identity, &me.display_name)?;
        // 前の割り当ての後始末は、新しい割り当てと同じトランザクションで記録する（退出と同じ worker が処理する）。
        let departed = timed_sql(
            t,
            "record_previous",
            record_previous_assignment(&mut tx, &channel_id, body.session_id, &me, Some(&identity)),
        )
        .await?;
        // 同じルームへの割り当てを出し直すだけなら、bridge が出しているトラックを残し、版も進めない。
        let changed = timed_sql(
            t,
            "set_assignment",
            set_assignment(
                &mut tx,
                &channel_id,
                body.session_id,
                &me,
                Assignment {
                    sfu_id: SFU_LIVEKIT,
                    room: Some(&room),
                    cf_session: None,
                    livekit_identity: Some(&identity),
                },
            ),
        )
        .await?;
        if changed {
            timed_sql(t, "bump_tracks", bump_tracks(&mut tx, &channel_id)).await?;
        }
        timed_sql(t, "commit", async { Ok(tx.commit().await?) }).await?;
        drop(conn);
        (
            JoinTarget::Livekit {
                url: lk.ws_url().to_string(),
                token,
            },
            departed,
        )
    } else {
        // Cloudflare の API は、ロックも接続も持ったまま呼ばない。
        timed_sql(t, "commit", async { Ok(tx.commit().await?) }).await?;
        drop(conn);
        let cf_session = sfu.cloudflare.new_session().await?;
        let mut conn = acquire(&db, t, "tx2").await?;
        let mut tx = begin(&mut conn, t, "begin2").await?;
        // ロックを取り直す。呼んでいるあいだに退出していたら、作ったセッションは使わない
        // （何も流れないので放置してよい）。割り当ても、ロックを外していたあいだに変わりうるので読み直す。
        lock_channel(&mut tx, &channel_id, t, "lock2").await?;
        let me = timed_sql(
            t,
            "own_presence2",
            own_presence(&mut tx, &channel_id, body.session_id, &user.id, true),
        )
        .await?;
        if me.epoch != Some(epoch) || me.mode != Some(RoomMode::Sfu) {
            return Err(AppError::Conflict("Call room changed while joining".into()));
        }
        let departed = timed_sql(
            t,
            "record_previous2",
            record_previous_assignment(&mut tx, &channel_id, body.session_id, &me, None),
        )
        .await?;
        // Cloudflare のセッションは毎回新しいので、割り当ては必ず変わる。
        let changed = timed_sql(
            t,
            "set_assignment2",
            set_assignment(
                &mut tx,
                &channel_id,
                body.session_id,
                &me,
                Assignment {
                    sfu_id: SFU_CLOUDFLARE,
                    room: None,
                    cf_session: Some(&cf_session),
                    livekit_identity: None,
                },
            ),
        )
        .await?;
        let cleared = timed_sql(
            t,
            "clear_stale_bridge",
            clear_bridge_tracks_without_cloudflare(&mut tx, &channel_id, body.session_id),
        )
        .await?;
        if changed || cleared {
            timed_sql(t, "bump_tracks2", bump_tracks(&mut tx, &channel_id)).await?;
        }
        // ロックを外していたあいだに LiveKit に入った人も数えるため、取り直したロックの下で数え直す。
        let counts = timed_sql(
            t,
            "count_others2",
            count_others(&mut tx, &channel_id, body.session_id),
        )
        .await?;
        if !sfu.bridge_enabled() && counts.livekit > 0 {
            // bridge がないので、LiveKit の人にも Cloudflare に移ってもらう。
            let event = CallDistributedEvent::SfuReassign {
                channel_id: channel_id.clone(),
                epoch,
            };
            timed_sql(t, "notify", notify_in(&mut tx, &event)).await?;
        }
        timed_sql(t, "commit2", async { Ok(tx.commit().await?) }).await?;
        (JoinTarget::Cloudflare, departed)
    };
    if departed {
        sfu.departures_wake.notify_one();
    }

    let loaded = if t.is_some() {
        // 計測：トラック一覧の読み取りも、接続の取得と SQL を分けて測る。
        let mut conn = acquire(&db, t, "tracks").await?;
        timed_sql(t, "tracks", load_sfu_tracks(&mut conn, &channel_id)).await?
    } else {
        load_sfu_tracks(&mut db, &channel_id).await?
    };
    // 割り当てのコミットのあと、一覧を読む前に全員が退出して次の通話（新しい epoch）が始まっていたら、
    // 接続先（前の epoch のルーム）と一覧が食い違う。混ぜて返さず、競合として返す（クライアントは
    // 新しい room_state から入り直す）。
    if let Some((loaded_epoch, ..)) = &loaded
        && *loaded_epoch != epoch
    {
        return Err(AppError::Conflict("Call room changed while joining".into()));
    }
    let (epoch, version, tracks) = loaded.unwrap_or((epoch, 0, Vec::new()));
    Ok(Json(JoinResponse {
        target,
        epoch,
        version,
        tracks,
    }))
}

/// 接続のプールから接続を取る（計測するときは、`name` で取得の時間を残す）。
async fn acquire(
    db: &toasty::Db,
    timings: Option<&JoinTimingKey>,
    name: &str,
) -> Result<toasty::Connection, AppError> {
    let started = std::time::Instant::now();
    let mut conn = db.connection().await?;
    if let Some(key) = timings {
        let elapsed = started.elapsed();
        key.acquire_time(name, elapsed);
        probe_connection(&mut conn, key, name, elapsed).await?;
    }
    Ok(conn)
}

/// 計測：取った接続が、この取得で新しく開いたものかを推定する。
///
/// プールが接続を開いた時点の情報は toasty から取れないので、次の 2 つがそろったときに「新しい接続（推定）」とする。
/// - その接続のサーバー側のプロセス（`pg_backend_pid()`）を、計測した取得で初めて見た。
/// - そのプロセスの年齢（`backend_start` から）が、取得にかかった時間（と少しの余裕）以下。
///
/// 1 つ目で、ほかの計測した要求が少し前に開いて返した接続を、使い回したときに新しいと数えないようにする。
/// 計測しない要求（SSE など）が取得のあいだに開いて返した接続は、区別できない（`join_latency` では、計測中に
/// そのような要求は出さない）。調べる SQL の時間は `probe_{name}` として SQL に残す。
async fn probe_connection(
    conn: &mut toasty::Connection,
    key: &JoinTimingKey,
    name: &str,
    acquire: Duration,
) -> Result<(), AppError> {
    static SEEN_BACKENDS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::LazyLock::new(Default::default);
    let started = std::time::Instant::now();
    let rows = toasty::sql::query(
        "SELECT json_build_object(
                  'pid', pg_backend_pid(),
                  'age_us', (extract(epoch FROM clock_timestamp() - backend_start) * 1000000)::bigint
                )::text
         FROM pg_stat_activity WHERE pid = pg_backend_pid()",
    )
    .exec(&mut *conn)
    .await?;
    key.sql_time(&format!("probe_{name}"), started.elapsed());
    let probe: Option<serde_json::Value> =
        first_text(rows).and_then(|v| serde_json::from_str(&v).ok());
    let pid = probe.as_ref().map(|p| p["pid"].to_string());
    let age = probe
        .as_ref()
        .and_then(|p| p["age_us"].as_u64())
        .map(Duration::from_micros);
    let first_seen = pid.is_some_and(|pid| {
        SEEN_BACKENDS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pid)
    });
    let likely_new_connection = first_seen
        && age.is_some_and(|age| age <= acquire + started.elapsed() + Duration::from_millis(5));
    key.push(|t| {
        t.acquires.push(AcquireEvent {
            name: name.to_string(),
            elapsed: acquire,
            likely_new_connection,
        })
    });
    Ok(())
}

/// 計測：`AuthUser` の取り出しのときの、有効なユーザーかの確認（`apps::events::active_user` と同じ）を、
/// 接続の取得（`auth_user`）、BEGIN（`auth_user_begin`）、SQL（`auth_user`）に分けて測りながら行う。
pub(crate) async fn timed_active_user(
    state: &AppState,
    key: &JoinTimingKey,
    user: &str,
) -> Result<(), AppError> {
    let mut conn = acquire(&state.db, Some(key), "auth_user").await?;
    let mut tx = begin(&mut conn, Some(key), "auth_user_begin").await?;
    timed_sql(
        Some(key),
        "auth_user",
        crate::apps::events::active_user_tx(&mut tx, user),
    )
    .await
}

/// 取った接続でトランザクションを始める（計測するときは、`name` で BEGIN の往復の時間を残す）。
async fn begin<'a>(
    conn: &'a mut toasty::Connection,
    timings: Option<&JoinTimingKey>,
    name: &str,
) -> Result<toasty::Transaction<'a>, AppError> {
    let started = std::time::Instant::now();
    let tx = conn.transaction().await?;
    if let Some(key) = timings {
        key.sql_time(name, started.elapsed());
    }
    Ok(tx)
}

/// SQL を 1 つ実行し、計測するときは `name` でその時間を残す。計測しないときは、そのまま待つだけ。
async fn timed_sql<T>(
    timings: Option<&JoinTimingKey>,
    name: &str,
    fut: impl std::future::Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    let Some(key) = timings else {
        return fut.await;
    };
    let started = std::time::Instant::now();
    let result = fut.await;
    key.sql_time(name, started.elapsed());
    result
}

/// チャンネル行のロック。入室と SFU の割り当てを、このロックで直列化する。
///
/// 計測するときは、ロックの文を `name`、そのあと同じ接続での `SELECT 1`（SQL の往復だけ）を
/// `{name}_rtt` で残し、差をロック待ちの目安（`lock_wait`）にする。
async fn lock_channel(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    timings: Option<&JoinTimingKey>,
    name: &str,
) -> Result<(), AppError> {
    let started = std::time::Instant::now();
    toasty::sql::query("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel_id.to_string())
        .exec(&mut *tx)
        .await?;
    if let Some(key) = timings {
        let lock = started.elapsed();
        let rtt_started = std::time::Instant::now();
        toasty::sql::query("SELECT 1").exec(&mut *tx).await?;
        let rtt = rtt_started.elapsed();
        key.sql_time(name, lock);
        key.sql_time(&format!("{name}_rtt"), rtt);
        key.push(|t| t.lock_wait.push(lock.saturating_sub(rtt)));
    }
    Ok(())
}

/// テスト用（join の計測）：join の各段階にかかった時間を、要求ごとの記録先に残す。開発用の認証が有効な
/// ときだけ、`X-Test-Join-Timings` ヘッダーで記録先の名前を指定できる。名前ごとに分けるので、同じプロセスで
/// 並行に動くほかのテストの要求は混ざらない。ヘッダーがなければ、どの段階も記録しない（本番の処理は変わらない）。
pub(crate) struct JoinTimingKey(String);

/// 計測：接続の取得 1 回分。
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct AcquireEvent {
    /// 段階の名前（`acquire` の名前と同じ）。
    pub name: String,
    /// 取得にかかった時間。
    pub elapsed: Duration,
    /// この取得のあいだに、新しい接続を開いたか（推定。`probe_connection` の条件）。
    pub likely_new_connection: bool,
}

/// 記録先ごとの時間。
#[doc(hidden)]
#[derive(Default, Debug, Clone)]
pub struct JoinTimings {
    /// 接続のプールからの取得（空きを待つ時間を含む）。段階の名前ごと。
    pub acquire: std::collections::BTreeMap<String, Vec<Duration>>,
    /// SQL の実行（BEGIN・ロック・COMMIT を含む）。段階の名前ごと。
    pub sql: std::collections::BTreeMap<String, Vec<Duration>>,
    /// ロックの文から、同じ接続での `SELECT 1` を引いたもの（ロック待ちの目安）。
    pub lock_wait: Vec<Duration>,
    /// 接続の取得ごとの、時間と、新しい接続を開いたか（推定）。
    pub acquires: Vec<AcquireEvent>,
    /// 要求を送ってから、handler が始まるまで（`X-Test-Join-Sent-At` があるとき）。ルーティング、本文の読み取り、
    /// `AuthUser` の取り出し（ユーザーの解決と有効なユーザーかの確認。DB を読む）を含む。
    pub pre_handler: Vec<Duration>,
}

static JOIN_TIMINGS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, JoinTimings>>,
> = std::sync::LazyLock::new(Default::default);

impl JoinTimingKey {
    /// `AuthUser` の取り出しで使う。開発用の認証のときだけヘッダーを見る（本番では見ない）。
    pub(crate) fn for_extractor(state: &AppState, headers: &HeaderMap) -> Option<Self> {
        if !state.config.dev_auth_enabled {
            return None;
        }
        headers
            .get("X-Test-Join-Timings")
            .and_then(|v| v.to_str().ok())
            .map(|k| Self(k.to_string()))
    }

    fn from_headers(state: &AppState, headers: &HeaderMap) -> Option<Self> {
        if !state.config.dev_auth_enabled {
            return None;
        }
        let key = Self(
            headers
                .get("X-Test-Join-Timings")
                .and_then(|v| v.to_str().ok())?
                .to_string(),
        );
        // 送った時刻（UNIX 時刻のマイクロ秒）から、handler が始まるまでの時間。
        let sent = headers
            .get("X-Test-Join-Sent-At")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if let Some(sent) = sent {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_micros() as u64)
                .unwrap_or(0);
            let elapsed = Duration::from_micros(now.saturating_sub(sent));
            key.push(|t| t.pre_handler.push(elapsed));
        }
        Some(key)
    }

    fn acquire_time(&self, name: &str, elapsed: Duration) {
        self.push(|t| t.acquire.entry(name.to_string()).or_default().push(elapsed));
    }

    fn sql_time(&self, name: &str, elapsed: Duration) {
        self.push(|t| t.sql.entry(name.to_string()).or_default().push(elapsed));
    }

    /// 記録先が登録されているとき（`register_join_timings_for_test`）だけ書く。登録のない名前
    /// （`sfu/join` 以外の要求や、回収しない要求に付いたヘッダー）は何も残さない。
    fn push(&self, f: impl FnOnce(&mut JoinTimings)) {
        let mut map = JOIN_TIMINGS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = map.get_mut(&self.0) {
            f(t);
        }
    }
}

/// テスト用（join の計測）：記録先 `key` を登録する。返した値を落とすと（join の失敗・キャンセルを含む）、
/// 記録先は消える。成功したら `take` で取り出す。
#[doc(hidden)]
pub fn register_join_timings_for_test(key: &str) -> JoinTimingsCollector {
    JOIN_TIMINGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), JoinTimings::default());
    JoinTimingsCollector(key.to_string())
}

/// テスト用（join の計測）：登録した記録先。落とすと消える。
#[doc(hidden)]
pub struct JoinTimingsCollector(String);

impl JoinTimingsCollector {
    /// 残した時間を取り出し、記録先を消す。
    pub fn take(self) -> JoinTimings {
        JOIN_TIMINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0)
            .unwrap_or_default()
    }
}

impl Drop for JoinTimingsCollector {
    fn drop(&mut self) {
        JOIN_TIMINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

#[derive(Deserialize, Default)]
struct Counts {
    cloudflare: i64,
    livekit: i64,
}

/// 自分以外の在室者を、割り当て先ごとに数える。探索は call_presence の主キーの先頭（channel_id）で、
/// その部屋の在室者（定員以下）だけ。
async fn count_others(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    session_id: Uuid,
) -> Result<Counts, AppError> {
    let rows = toasty::sql::query(
        "SELECT json_build_object(
             'cloudflare', count(*) FILTER (WHERE sfu_id = 'cloudflare'),
             'livekit', count(*) FILTER (WHERE sfu_id = 'livekit'))::text
         FROM call_presence
         WHERE channel_id = $1 AND session_id <> $2::uuid AND expires_at > statement_timestamp()",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .exec(&mut *tx)
    .await?;
    Ok(rows_json::<Counts>(rows)
        .into_iter()
        .next()
        .unwrap_or_default())
}

/// 前の割り当て（fallback・やり直し）の後始末を `call_sfu_departures` に記録する。新しい割り当てと同じ
/// トランザクションで呼ぶので、どちらかだけが残ることはない。記録したら `true`。
///
/// `new_livekit_identity` は、新しい割り当てが LiveKit のときの identity。前と同じ identity（同じルームへの
/// join の再送）なら、つないでいる接続を切ってしまうので、LiveKit の後始末はしない。記録するのは前の
/// identity で、worker はその identity だけを外す。
async fn record_previous_assignment(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    session_id: Uuid,
    previous: &PresenceRow,
    new_livekit_identity: Option<&str>,
) -> Result<bool, AppError> {
    let previous_on_livekit = previous.sfu_id.as_deref() == Some(SFU_LIVEKIT)
        && (previous.livekit_identity.is_none()
            || previous.livekit_identity.as_deref() != new_livekit_identity);
    let previous_livekit_room = previous.sfu_room.as_deref().filter(|_| previous_on_livekit);
    let previous_cf = previous.cf_session_id.as_deref();
    if previous_livekit_room.is_none() && previous_cf.is_none() {
        return Ok(false);
    }
    toasty::sql::statement(
        "INSERT INTO call_sfu_departures
           (channel_id, session_id, sfu_id, sfu_room, cf_session_id, livekit_identity)
         VALUES ($1, $2::uuid, $3, NULLIF($4, ''), NULLIF($5, ''), NULLIF($6, ''))",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .bind(if previous_livekit_room.is_some() {
        SFU_LIVEKIT
    } else {
        SFU_CLOUDFLARE
    })
    .bind(previous_livekit_room.unwrap_or_default().to_string())
    .bind(previous_cf.unwrap_or_default().to_string())
    .bind(
        previous_livekit_room
            .and(previous.livekit_identity.as_deref())
            .unwrap_or_default()
            .to_string(),
    )
    .exec(&mut *tx)
    .await?;
    Ok(true)
}

/// 割り当てを変える。呼ぶ前に、同じトランザクションで presence の行をロックしておく（`own_presence` の
/// `lock`）。`register_track` も同じ行をロックしてから登録するので、古い割り当てで登録したトラックが、
/// ここで消したあとに残ることはない。
///
/// `previous` は、ロックして読んだ今の割り当て。新しい割り当てと同じなら（同じ LiveKit のルームへの
/// join の再送）、何もしない。トラックを消すと、LiveKit の購読が変わらない bridge は push し直さないので、
/// あとから来た Cloudflare の人がその話者を pull できなくなる。割り当てを変えたら `true`。
async fn set_assignment(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    session_id: Uuid,
    previous: &PresenceRow,
    new: Assignment<'_>,
) -> Result<bool, AppError> {
    let Assignment {
        sfu_id,
        room,
        cf_session,
        livekit_identity,
    } = new;
    if previous.sfu_id.as_deref() == Some(sfu_id)
        && previous.sfu_room.as_deref() == room
        && previous.cf_session_id.as_deref() == cf_session
        && previous.livekit_identity.as_deref() == livekit_identity
    {
        return Ok(false);
    }
    toasty::sql::statement(
        "UPDATE call_presence SET sfu_id = $3, sfu_room = NULLIF($4, ''), cf_session_id = NULLIF($5, ''),
             livekit_identity = NULLIF($6, '')
         WHERE channel_id = $1 AND session_id = $2::uuid",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .bind(sfu_id.to_string())
    // toasty は NULL の型を推定できないので、空文字を NULL として渡す。
    .bind(room.unwrap_or_default().to_string())
    .bind(cf_session.unwrap_or_default().to_string())
    .bind(livekit_identity.unwrap_or_default().to_string())
    .exec(&mut *tx)
    .await?;
    // 割り当てが変わったら、前の SFU に出していたトラックは消す。
    toasty::sql::statement(
        "DELETE FROM call_sfu_tracks WHERE channel_id = $1 AND session_id = $2::uuid",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .exec(&mut *tx)
    .await?;
    Ok(true)
}

/// 新しい割り当て（`set_assignment`）。
struct Assignment<'a> {
    sfu_id: &'a str,
    room: Option<&'a str>,
    cf_session: Option<&'a str>,
    livekit_identity: Option<&'a str>,
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

/// テスト用の一時停止。開発用の認証が有効なときだけ、ヘッダーで指定できる（`routes::messages` の
/// test gate と同じヘッダー）。`X-Test-Gate-Reached` の gate に到着を知らせ、`X-Test-Gate-Wait` の gate が
/// 開くまで（最長 `X-Test-Gate-Wait-Ms`、既定 1000 ミリ秒）待つ。競合の順序を固定するテストに使う。
struct PauseGate {
    reached: Option<String>,
    wait: String,
    timeout_ms: u64,
}

impl PauseGate {
    fn from_headers(state: &AppState, headers: &HeaderMap) -> Option<Self> {
        if !state.config.dev_auth_enabled {
            return None;
        }
        let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        header("X-Test-Gate-Wait").map(|wait| Self {
            wait: wait.to_string(),
            reached: header("X-Test-Gate-Reached").map(str::to_string),
            timeout_ms: header("X-Test-Gate-Wait-Ms")
                .and_then(|ms| ms.parse::<u64>().ok())
                .map_or(1000, |ms| ms.min(30_000)),
        })
    }

    async fn pause(gate: Option<&Self>) {
        let Some(gate) = gate else {
            return;
        };
        if let Some(reached) = &gate.reached {
            crate::routes::messages::test_gate(reached).notify_one();
        }
        let wait = crate::routes::messages::test_gate(&gate.wait);
        let _ = tokio::time::timeout(Duration::from_millis(gate.timeout_ms), wait.notified()).await;
    }
}

/// マイクの音声を Cloudflare に push する。SDP は音声 1 本だけ。
async fn cf_publish(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PublishBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = sfu(&state)?;
    let cf = own_cf_session(&state, &user, &channel_id, body.session_id).await?;
    let mid = crate::call_sfu::cloudflare::validate_audio_only_offer(&body.offer)?;
    let track_name = body.session_id.to_string();
    let res = match sfu
        .cloudflare
        .push_track(&cf, &body.offer, &mid, &track_name)
        .await
    {
        Ok(res) => res,
        Err(e) => {
            // 応答がない・失敗した push も、Cloudflare では済んでいることがある。結果が決まったあとで閉じ直す。
            record_unknown_cf_outcome(&state, sfu, &channel_id, body.session_id, &cf).await;
            return Err(e);
        }
    };
    // Cloudflare が受け付けなかったトラックは、一覧に載せない。
    crate::call_sfu::cloudflare::check_pushed(&res, &mid)?;
    // テスト：Cloudflare の応答のあと、登録の前で止める（そのあいだに割り当てを変える）。
    PauseGate::pause(PauseGate::from_headers(&state, &headers).as_ref()).await;
    let registered = register_track(
        &state,
        &channel_id,
        body.session_id,
        TrackSource::Direct,
        &cf,
        &track_name,
        None,
    )
    .await;
    if let Err(e @ (AppError::NotFound(_) | AppError::Conflict(_))) = registered {
        // push のあいだに退出・割り当ての変更があった。push で作ったトラックを閉じ直す。
        record_cf_departure_after_op(&state, sfu, &channel_id, body.session_id, &cf, 0).await?;
        return Err(e);
    }
    registered?;
    Ok(Json(res))
}

/// Cloudflare の操作（push・pull）が終わったあと、その Cloudflare セッションがまだこの人の割り当てかを
/// 確かめる。違えば（`false`）、同じセッションの後始末を記録し直す。
///
/// 退出の後始末（worker の `close_session`）は、その時点の `GET` に載った mid だけを閉じる。操作が
/// Cloudflare で処理中のあいだに後始末が走ると、操作の結果が載る前の一覧で閉じ終えてしまう
/// （https://developers.cloudflare.com/realtime/sfu/api/）。操作の応答を受け取ったあとに記録し直せば、
/// worker の次の `GET` には操作の結果が載っている。まだ割り当てのままなら、このあとの退出は、操作の
/// 結果が載った一覧で閉じる。探索：call_presence の主キーで 1 行。
async fn still_assigned_to_cf(
    state: &AppState,
    sfu: &CallSfu,
    channel_id: &str,
    session_id: Uuid,
    cf: &str,
) -> Result<bool, AppError> {
    let mut db = state.db.clone();
    let rows = toasty::sql::query(
        "SELECT 1 FROM call_presence
         WHERE channel_id = $1 AND session_id = $2::uuid AND sfu_id = 'cloudflare'
           AND cf_session_id = $3 AND expires_at > statement_timestamp()",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .bind(cf.to_string())
    .exec(&mut db)
    .await?;
    if !rows.is_empty() {
        return Ok(true);
    }
    record_cf_departure_after_op(state, sfu, channel_id, session_id, cf, 0).await?;
    Ok(false)
}

/// Cloudflare の操作（push・pull）が失敗した・応答がなかったときの後始末。
///
/// 結果はわからない。Cloudflare では操作が済んでいる、またはこのあと済むことがある（応答を待つのをやめても、
/// Cloudflare 側の処理は取り消されない）。そのあいだに退出の後始末が `GET` した一覧には、操作の結果が
/// 載らない。そこで、操作の結果が決まったあと（`UNKNOWN_OUTCOME_RECLOSE_SECS` の 30 秒後と 5 分後）に閉じ直す
/// 行を記録する。
/// worker はその時点でセッションがまだその人の割り当てなら閉じない（`Departure::cf_still_assigned`）ので、
/// 通話を続けている人のトラックは切らない。記録に失敗しても、元のエラーを返す（ログに残す）。
async fn record_unknown_cf_outcome(
    state: &AppState,
    sfu: &CallSfu,
    channel_id: &str,
    session_id: Uuid,
    cf: &str,
) {
    for delay in UNKNOWN_OUTCOME_RECLOSE_SECS {
        if let Err(e) =
            record_cf_departure_after_op(state, sfu, channel_id, session_id, cf, delay).await
        {
            tracing::error!(error = ?e, channel_id, %session_id, "結果のわからない Cloudflare の操作の後始末を記録できませんでした");
        }
    }
}

/// 結果のわからない操作のあと、閉じ直すまでの秒数。1 回目は Cloudflare への要求のタイムアウト（10 秒）より
/// 長くする。Cloudflare は処理時間の上限を示していないので、遅れて済んだ場合に備えて 5 分後にもう一度閉じ直す。
const UNKNOWN_OUTCOME_RECLOSE_SECS: [i32; 2] = [30, 300];

/// Cloudflare のセッションの後始末を、`call_sfu_departures` に記録し直す（`still_assigned_to_cf`）。
/// `delay_secs` が 0 より大きければ、worker はその秒数が過ぎてから処理する（`claimed_until`）。
async fn record_cf_departure_after_op(
    state: &AppState,
    sfu: &CallSfu,
    channel_id: &str,
    session_id: Uuid,
    cf: &str,
    delay_secs: i32,
) -> Result<(), AppError> {
    let mut db = state.db.clone();
    toasty::sql::statement(
        "INSERT INTO call_sfu_departures (channel_id, session_id, sfu_id, cf_session_id, claimed_until)
         VALUES ($1, $2::uuid, 'cloudflare', $3,
                 CASE WHEN $4::int > 0 THEN statement_timestamp() + make_interval(secs => $4::int) END)",
    )
    .bind(channel_id.to_string())
    .bind(session_id.to_string())
    .bind(cf.to_string())
    .bind(delay_secs.to_string())
    .exec(&mut db)
    .await?;
    sfu.departures_wake.notify_one();
    Ok(())
}

/// 1 列目が text の行を 1 つ読む。
fn first_text(rows: Vec<toasty::stmt::Value>) -> Option<String> {
    rows.into_iter().find_map(|row| match row {
        toasty::stmt::Value::Record(rec) => match rec.fields.into_iter().next() {
            Some(toasty::stmt::Value::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    })
}

/// 登録するトラックの種類と、登録してよい条件。
enum TrackSource {
    /// 話者のクライアントが push したもの。話者の割り当てが、push に使った Cloudflare セッションのままのこと。
    Direct,
    /// LiveKit の話者を bridge が push したもの。話者がまだ LiveKit にいること。
    /// `publish_id` は bridge が push ごとに付ける ID。close はこの ID の行だけを外す（同じ話者を push し直した
    /// 新しい行を、遅れて届いた前の close が外さないように）。
    Bridge {
        bridge_epoch: i64,
        publish_id: String,
    },
}

/// push したトラックを一覧に載せる。
///
/// 話者の presence の行をロックして、割り当てを確かめてから書く。`set_assignment` も同じ行をロックするので、
/// push のあいだに割り当てが変わっていれば 409 を返し、古いセッションのトラックを載せない
/// （古いセッションは、割り当ての変更で `call_sfu_departures` に記録され、worker が閉じる）。
async fn register_track(
    state: &AppState,
    channel_id: &str,
    speaker: Uuid,
    source: TrackSource,
    cf_session: &str,
    track_name: &str,
    pause: Option<&PauseGate>,
) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 探索：call_presence の主キーで 1 行。
    let rows = toasty::sql::query(
        "SELECT json_build_object('sfu_id', sfu_id, 'cf_session_id', cf_session_id)::text
         FROM call_presence
         WHERE channel_id = $1 AND session_id = $2::uuid AND expires_at > statement_timestamp()
         FOR UPDATE",
    )
    .bind(channel_id.to_string())
    .bind(speaker.to_string())
    .exec(&mut tx)
    .await?;
    #[derive(Deserialize)]
    struct Assignment {
        sfu_id: Option<String>,
        cf_session_id: Option<String>,
    }
    let Some(current) = rows_json::<Assignment>(rows).into_iter().next() else {
        return Err(AppError::NotFound("Call session not found".into()));
    };
    let (source_name, bridge_epoch, publish_id) = match source {
        TrackSource::Direct => {
            if current.sfu_id.as_deref() != Some(SFU_CLOUDFLARE)
                || current.cf_session_id.as_deref() != Some(cf_session)
            {
                return Err(AppError::Conflict(
                    "The SFU assignment changed while publishing".into(),
                ));
            }
            ("direct", None, None)
        }
        TrackSource::Bridge {
            bridge_epoch,
            publish_id,
        } => {
            if current.sfu_id.as_deref() != Some(SFU_LIVEKIT) {
                return Err(AppError::Conflict("Speaker is not on LiveKit".into()));
            }
            // 新しい bridge が登録されたあとの、古い bridge の push は載せない。登録（`bridge_register`）は
            // この行を更新するので、FOR SHARE で登録と直列にする。探索：主キーで 1 行。
            let rows = toasty::sql::query(
                "SELECT epoch::text FROM call_sfu_bridge WHERE id = 1 FOR SHARE",
            )
            .exec(&mut tx)
            .await?;
            if first_text(rows).and_then(|e| e.parse::<i64>().ok()) != Some(bridge_epoch) {
                return Err(AppError::Conflict(
                    "A newer sfu-bridge is registered".into(),
                ));
            }
            // テスト：epoch を確かめたあと、FOR SHARE を持ったまま、書く前で止める。
            PauseGate::pause(pause).await;
            ("bridge", Some(bridge_epoch), Some(publish_id))
        }
    };
    toasty::sql::statement(
        "INSERT INTO call_sfu_tracks
           (channel_id, session_id, source, cf_session_id, track_name, bridge_epoch, bridge_publish_id)
         VALUES ($1, $2::uuid, $3, $4, $5, NULLIF($6, '')::bigint, NULLIF($7, ''))
         ON CONFLICT (channel_id, session_id, source) DO UPDATE
           SET cf_session_id = EXCLUDED.cf_session_id, track_name = EXCLUDED.track_name,
               bridge_epoch = EXCLUDED.bridge_epoch, bridge_publish_id = EXCLUDED.bridge_publish_id",
    )
    .bind(channel_id.to_string())
    .bind(speaker.to_string())
    .bind(source_name.to_string())
    .bind(cf_session.to_string())
    .bind(track_name.to_string())
    .bind(bridge_epoch.map(|e| e.to_string()).unwrap_or_default())
    .bind(publish_id.unwrap_or_default())
    .exec(&mut tx)
    .await?;
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
    // 話者の presence もトラックごとに 1 行ずつ引き、一覧と同じく、期限の切れた（まだ消されていない）話者の
    // トラックは pull させない。
    let mut db = state.db.clone();
    let rows = toasty::sql::query(
        "SELECT json_build_object('session_id', t.cf_session_id, 'track_name', t.track_name)::text
         FROM jsonb_array_elements($2::jsonb) k
         JOIN call_sfu_tracks t
           ON t.channel_id = $1 AND t.session_id = (k->>0)::uuid AND t.source = k->>1
         -- LIMIT で結合への展開を止め、トラックごとに引く（全体の在室者を期限の索引で読まない）。
         CROSS JOIN LATERAL (
             SELECT 1 FROM call_presence p
             WHERE p.channel_id = t.channel_id AND p.session_id = t.session_id
               AND p.expires_at > statement_timestamp()
             LIMIT 1) live",
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
    headers: HeaderMap,
    Json(body): Json<PullBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = sfu(&state)?;
    let cf = own_cf_session(&state, &user, &channel_id, body.session_id).await?;
    let remotes =
        resolve_tracks(&state, &channel_id, &body.track_ids, Some(body.session_id)).await?;
    if remotes.is_empty() {
        return Err(AppError::NotFound("No tracks to pull".into()));
    }
    let res = match sfu.cloudflare.pull_tracks(&cf, &remotes).await {
        Ok(res) => res,
        Err(e) => {
            // 結果のわからない pull。クライアントはこのセッションを捨てて接続し直す（同じセッションで
            // pull し直すと購読が重なる）。Cloudflare で済んでいた購読は、結果が決まったあとで閉じ直す。
            record_unknown_cf_outcome(&state, sfu, &channel_id, body.session_id, &cf).await;
            return Err(e);
        }
    };
    // テスト：Cloudflare の応答のあと、割り当ての確認の前で止める（そのあいだに割り当てを変える）。
    PauseGate::pause(PauseGate::from_headers(&state, &headers).as_ref()).await;
    // pull のあいだに退出していれば、pull で作った購読を閉じ直す（退出した人に音声を流さない）。
    if !still_assigned_to_cf(&state, sfu, &channel_id, body.session_id, &cf).await? {
        return Err(AppError::Conflict(
            "The SFU assignment changed while pulling".into(),
        ));
    }
    Ok(Json(res))
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
        return Err(AppError::BadRequest(
            "sessionDescription must be an answer".into(),
        ));
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
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 最新の epoch を記録する。古い bridge の `register_track` は、この行の FOR SHARE を持ったまま書くので、
    // ここはその commit を待つ。そのあとの DELETE は新しいスナップショットで読むので、待った間の行も消せる。
    // epoch は時刻だが、時計が戻っても前より大きくする。探索：主キーで 1 行。
    let rows = toasty::sql::query(
        "INSERT INTO call_sfu_bridge (id, epoch)
         VALUES (1, (extract(epoch FROM clock_timestamp()) * 1000)::bigint)
         ON CONFLICT (id) DO UPDATE
           SET epoch = GREATEST(call_sfu_bridge.epoch + 1, EXCLUDED.epoch)
         RETURNING epoch::text",
    )
    .exec(&mut tx)
    .await?;
    let epoch: i64 = first_text(rows)
        .and_then(|e| e.parse().ok())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("sfu-bridge epoch was not recorded")))?;
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
    .exec(&mut tx)
    .await?;
    let _ = rows;
    tx.commit().await?;
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
        // 部屋は LiveKit の在室者から決め、call_rooms は主キーで 1 行ずつ引く（call_rooms は通話したことのある
        // チャンネルの数だけ増えるので、mode で全件を絞らない）。
        "WITH rooms AS MATERIALIZED (
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
                     'session_id', t.session_id, 'source', t.source,
                     'rev', left(md5(t.cf_session_id || '/' || t.track_name || '/' || COALESCE(t.bridge_publish_id, '')), 12))
                     ORDER BY t.session_id)
                 FROM call_sfu_tracks t
                 CROSS JOIN LATERAL (
                     SELECT 1 FROM call_presence tp
                     WHERE tp.channel_id = t.channel_id AND tp.session_id = t.session_id
                       AND tp.expires_at > statement_timestamp()
                     LIMIT 1) live
                 WHERE t.channel_id = r.channel_id AND t.source = 'direct'), '[]'::json)
         )::text
         FROM rooms
         CROSS JOIN LATERAL (
             SELECT cr.channel_id, cr.epoch FROM call_rooms cr
             WHERE cr.channel_id = rooms.channel_id AND cr.mode = 'sfu'
             LIMIT 1
         ) r
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
    let session_id = sfu.cloudflare.new_session().await?;
    remove_stale_bridge_tracks(&state, &body.channel_id).await?;
    Ok(Json(json!({ "cf_session_id": session_id })))
}

/// 部屋で最初の Cloudflare の人として入るとき、その部屋の bridge の行を一覧から外す。Cloudflare の人が
/// いない部屋では bridge は中継を止めているので、残っている行は止まった中継のもの（外す close を送れないまま
/// 止まった、など）。入ってくる人の join の応答と、そのあとの一覧に、古い行を配らない。新しい中継は、話者を
/// push し直して登録する。チャンネル行のロックの下で呼ぶ。
/// 探索：その部屋の在室（idx_call_presence_channel、部屋の人数以下）と、主キーの先頭でその部屋の bridge の行。
async fn clear_bridge_tracks_without_cloudflare(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    me: Uuid,
) -> Result<bool, AppError> {
    let rows = toasty::sql::query(
        "DELETE FROM call_sfu_tracks
         WHERE channel_id = $1 AND source = 'bridge'
           AND NOT EXISTS (
             SELECT 1 FROM call_presence
             WHERE channel_id = $1 AND session_id <> $2::uuid AND sfu_id = 'cloudflare'
               AND expires_at > statement_timestamp())
         RETURNING 1",
    )
    .bind(channel_id.to_string())
    .bind(me.to_string())
    .exec(&mut *tx)
    .await?;
    Ok(!rows.is_empty())
}

/// 部屋の中継を始め直すとき（bridge が新しい Cloudflare セッションを作るとき）、その部屋の bridge の行を
/// 一覧から外す。行は前の中継のセッションのもので、新しい中継は LiveKit の話者を push し直して登録する。
/// 前の中継が、一覧から外す close を送り直せないまま止まった（Cloudflare の人がいなくなった、など）ときの
/// 古い行も、ここで外れる。探索：主キー (channel_id, session_id, source) の先頭で、その部屋の行だけ。
async fn remove_stale_bridge_tracks(state: &AppState, channel_id: &str) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = toasty::sql::query(
        "DELETE FROM call_sfu_tracks WHERE channel_id = $1 AND source = 'bridge' RETURNING 1",
    )
    .bind(channel_id.to_string())
    .exec(&mut tx)
    .await?;
    if !rows.is_empty() {
        bump_tracks(&mut tx, channel_id).await?;
    }
    tx.commit().await?;
    Ok(())
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
    /// bridge が push ごとに付ける ID（close で、この push の行だけを外すため）。
    publish_id: String,
}

/// LiveKit の話者の声を、bridge の Cloudflare セッションから push する。
async fn bridge_cf_publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BridgePublishBody>,
) -> Result<Json<TracksResponse>, AppError> {
    let sfu = bridge_auth(&state, &headers)?;
    crate::call_sfu::cloudflare::validate_audio_offer_mid(&body.offer, &body.mid)?;
    if body.publish_id.is_empty() || body.publish_id.len() > 64 {
        return Err(AppError::BadRequest("Invalid publish_id".into()));
    }
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
    crate::call_sfu::cloudflare::check_pushed(&res, &body.mid)?;
    register_track(
        &state,
        &body.channel_id,
        body.speaker_session_id,
        TrackSource::Bridge {
            bridge_epoch: body.bridge_epoch,
            publish_id: body.publish_id.clone(),
        },
        &body.cf_session_id,
        &track_name,
        PauseGate::from_headers(&state, &headers).as_ref(),
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
        return Err(AppError::BadRequest(
            "bridge pulls direct tracks only".into(),
        ));
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
        return Err(AppError::BadRequest(
            "sessionDescription must be an answer".into(),
        ));
    }
    sfu.cloudflare
        .renegotiate(&body.cf_session_id, &body.answer)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
struct BridgeCloseBody {
    /// 閉じる mid の部屋。あれば、push を閉じた話者のトラックを一覧から外す（古い bridge は送らない）。
    #[serde(default)]
    channel_id: Option<String>,
    cf_session_id: String,
    mids: Vec<String>,
    /// `mids` のうち bridge の push を閉じるもの（話者の session_id と push の ID。pull だけなら空）。
    #[serde(default)]
    pushes: Vec<ClosedPush>,
}

#[derive(Debug, Deserialize)]
struct ClosedPush {
    speaker: Uuid,
    publish_id: String,
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
    if body.pushes.len() > MAX_PULL_TRACKS {
        return Err(AppError::BadRequest("Too many pushes".into()));
    }
    // 閉じる前に、push をやめる話者のトラックを一覧から外す。先に外すので、close が失敗しても一覧に
    // 古い行は残らない（mid はセッションを閉じれば消える）。一覧から外せなければ close もせずにエラーを返し、
    // bridge の push は残る（一覧と食い違わない）。話者は bridge が渡すので、Cloudflare に問い合わせない。
    if let Some(channel_id) = &body.channel_id {
        remove_closed_bridge_tracks(&state, channel_id, &body.cf_session_id, &body.pushes).await?;
    }
    sfu.cloudflare
        .close_tracks(&body.cf_session_id, &body.mids)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// bridge が閉じる push を、トラック一覧から外す。話者が LiveKit にいるまま公開をやめると、行が残り、
/// あとから来た Cloudflare の人に pull できないトラックが配られるため。
///
/// 外すのは、閉じたセッションと push の ID がいまの行と同じとき（その push の行）だけ。同じ話者を push し直した
/// 行は ID が違うので、遅れて届いた前の close でも外さない。
/// 探索：主キー (channel_id, session_id, source) で、閉じた push 1 つにつき 1 行。
async fn remove_closed_bridge_tracks(
    state: &AppState,
    channel_id: &str,
    cf_session: &str,
    pushes: &[ClosedPush],
) -> Result<(), AppError> {
    if pushes.is_empty() {
        return Ok(());
    }
    let speakers: Vec<String> = pushes.iter().map(|p| p.speaker.to_string()).collect();
    let ids = serde_json::to_string(
        &pushes
            .iter()
            .map(|p| p.publish_id.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|e| AppError::Internal(e.into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = toasty::sql::query(
        "DELETE FROM call_sfu_tracks t
         USING unnest(string_to_array($2, ',')::uuid[], ARRAY(SELECT json_array_elements_text($4::json)))
           AS p(speaker, publish_id)
         WHERE t.channel_id = $1 AND t.source = 'bridge'
           AND t.session_id = p.speaker AND t.cf_session_id = $3
           AND t.bridge_publish_id = p.publish_id
         RETURNING 1",
    )
    .bind(channel_id.to_string())
    .bind(speakers.join(","))
    .bind(cf_session.to_string())
    .bind(ids)
    .exec(&mut tx)
    .await?;
    if !rows.is_empty() {
        bump_tracks(&mut tx, channel_id).await?;
    }
    tx.commit().await?;
    Ok(())
}
