use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::AuthUser, db::get_current_txid, error::AppError, routes::response::ApiResponse,
    state::AppState,
};

/// 活動がこの日数ない（返信・再Openのどちらもない）スレッドを、自動でアーカイブする。
/// Slack の取り込みが、直近の返信かどうかを決めるのにも使う（`slack::is_live_reply_ts`）。
pub const THREAD_IDLE_DAYS: i64 = 1;

/// How long the participants read after a join / leave / unarchive may take (`statement_timeout`).
/// Shorter than the 10 s the Web client waits for the POST: a read that needs longer is left out of
/// the answer (the write is already committed) instead of making the client give up on the write.
pub const THREAD_PARTICIPANTS_READ_TIMEOUT_MS: u64 = 5_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads", get(list_threads))
        .route("/api/threads/{id}", get(thread_detail))
        .route("/api/threads/{id}/join", post(join_thread))
        .route("/api/threads/{id}/leave", post(leave_thread))
        .route("/api/threads/{id}/unarchive", post(unarchive_thread))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

#[derive(Default, Deserialize)]
struct ThreadQuery {
    channel_id: Option<String>,
    #[serde(default)]
    archived: bool,
    before: Option<String>,
    through: Option<String>,
    limit: Option<usize>,
}

async fn list_threads(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(page): Query<ThreadQuery>,
) -> Result<Json<Value>, AppError> {
    if page.archived {
        return Ok(Json(json!({"items": [], "next_cursor": null})));
    }
    let limit = page.limit.unwrap_or(100).clamp(1, 100);
    let parse_cursor = |value: &str| -> Result<[String; 2], AppError> {
        let cursor: [String; 2] = serde_json::from_str(value)
            .map_err(|_| AppError::BadRequest("Invalid thread cursor".into()))?;
        cursor[0]
            .parse::<jiff::Timestamp>()
            .map_err(|_| AppError::BadRequest("Invalid thread cursor".into()))?;
        Ok(cursor)
    };
    let cursor = page.before.as_deref().map(parse_cursor).transpose()?;
    let through = page.through.as_deref().map(parse_cursor).transpose()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let mut rows = crate::apps::query(
        &mut tx,
        "WITH joined_channels AS MATERIALIZED ( \
           SELECT c.id FROM channel_user cu JOIN channels c ON c.id=cu.channel_id \
           WHERE cu.user_id=$1 AND c.deleted_at IS NULL AND NOT c.is_dm AND ($2='' OR c.id=$2) \
         ), candidates AS ( \
           SELECT root.* FROM joined_channels c \
           CROSS JOIN LATERAL ( \
             SELECT root.id,root.channel_id,root.content,root.thread_archive_at,root.thread_activity_at AS activity_at \
             FROM messages root WHERE root.channel_id=c.id AND root.deleted_at IS NULL \
               AND root.thread_activity_at IS NOT NULL \
               AND root.thread_archive_at IS NULL \
               AND ((SELECT phase FROM thread_rollout WHERE id)<>'ready' OR root.parent_id IS NULL) \
               AND ((SELECT phase FROM thread_rollout WHERE id)<>'ready' OR EXISTS(SELECT 1 FROM thread_participants tp WHERE tp.thread_id=root.id AND tp.user_id=$1 AND tp.left_at IS NULL)) \
               AND (root.thread_activity_at,root.id)<((COALESCE(NULLIF($3,'')::timestamptz,'infinity'::timestamptz) AT TIME ZONE 'UTC'),$4) \
               AND (root.thread_activity_at,root.id)>=((COALESCE(NULLIF($6,'')::timestamptz,'-infinity'::timestamptz) AT TIME ZONE 'UTC'),$7) \
               AND EXISTS(SELECT 1 FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL) \
             ORDER BY root.thread_activity_at DESC,root.id DESC LIMIT $5::int \
           ) root \
         ), page AS ( \
           SELECT * FROM candidates ORDER BY activity_at DESC,id DESC LIMIT $5::int \
         ) SELECT json_build_object('id',root.id,'channel_id',root.channel_id, \
           'content',left(root.content,160),'thread_archive_at',root.thread_archive_at, \
           '_activity_at',root.activity_at, \
           'reply_count',(SELECT count(*) FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL))::text \
         FROM page root ORDER BY root.activity_at DESC,root.id DESC",
        &[
            auth_user.id.as_str(), page.channel_id.as_deref().unwrap_or(""),
            cursor.as_ref().map(|v| v[0].as_str()).unwrap_or(""),
            cursor.as_ref().map(|v| v[1].as_str()).unwrap_or(""),
            &(limit + 1).to_string(),
            through.as_ref().map(|v| v[0].as_str()).unwrap_or(""),
            through.as_ref().map(|v| v[1].as_str()).unwrap_or(""),
        ],
    ).await?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if more {
        rows.last()
            .and_then(|row| row["_activity_at"].as_str().zip(row["id"].as_str()))
            .map(|(at, id)| json!([format!("{at}Z"), id]).to_string())
    } else {
        None
    };
    for row in &mut rows {
        if let Some(object) = row.as_object_mut() {
            object.remove("_activity_at");
        }
    }
    tx.commit().await?;
    Ok(Json(json!({"items":rows,"next_cursor":next_cursor})))
}

// Membership is a personal subscription, not a read permission. Public-channel
// readers can inspect this state even when they have never joined the thread.
/// `?participants=none` leaves out the participant count and list, which cost O(P) to read.
/// Clients that never show them (Android, TUI, desktop) ask for that; `tracked` is always
/// answered. Anything else keeps the full answer.
#[derive(Default, Deserialize)]
struct DetailQuery {
    participants: Option<String>,
}

impl DetailQuery {
    fn with_participants(&self) -> bool {
        self.participants.as_deref() != Some("none")
    }
}

async fn thread_detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    thread_operation(state, auth, id, "read", query.with_participants()).await
}

async fn join_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    thread_operation(state, auth, id, "join", query.with_participants()).await
}

async fn leave_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    thread_operation(state, auth, id, "leave", query.with_participants()).await
}

async fn unarchive_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    thread_operation(state, auth, id, "unarchive", query.with_participants()).await
}

/// Whether the thread (`$1`) has any participant row, left ones included: the `tracked` of an
/// answer without the participant list. One index entry, as the reply gate reads it (see
/// `reply_participation_sql`); shared with the scale test.
pub fn thread_tracked_sql() -> &'static str {
    "SELECT json_build_object('tracked',(SELECT true FROM thread_participants WHERE thread_id=$1 ORDER BY user_id LIMIT 1) IS NOT NULL)::text"
}

/// Participants of one thread (`$1` = thread id, `$2` = its channel id): `tracked`, live count and
/// the first 50. Live = joined, not deactivated, and still a member of the channel (a participant
/// who left the channel keeps the thread row, but is not listed or counted: the list must not show
/// profiles beyond what the viewer's channels already reveal). Shared with the scale test so it
/// measures exactly the SQL the route runs.
pub fn participants_sql() -> &'static str {
    "WITH keys AS MATERIALIZED (SELECT p.user_id,p.joined_at,p.left_at FROM thread_participants p WHERE p.thread_id=$1), \
              live AS MATERIALIZED (SELECT u.id,u.user_name,u.display_name,u.avatar_url,k.joined_at FROM keys k CROSS JOIN LATERAL (SELECT x.id,x.user_name,x.display_name,x.avatar_url FROM users x WHERE x.id=k.user_id AND NOT x.is_deactivated AND (SELECT true FROM channel_user cu WHERE cu.user_id=x.id AND cu.channel_id=$2) IS NOT NULL OFFSET 0) u WHERE k.left_at IS NULL) \
         SELECT json_build_object('tracked',EXISTS(SELECT 1 FROM keys),'count',(SELECT count(*) FROM live),'items',COALESCE((SELECT json_agg(json_build_object('id',id,'user_name',user_name,'display_name',display_name,'avatar_url',avatar_url) ORDER BY joined_at,id) FROM (SELECT * FROM live ORDER BY joined_at,id LIMIT 50) top),'[]'::json))::text"
}

/// The participants of a thread, read in a transaction of their own (after a write committed).
/// - The viewer's access to the channel is checked again here, as a read: the write committed
///   before this read, and a membership can be revoked in between. Without access the read fails,
///   and the caller leaves the list out of the answer.
/// - Postgres cancels a statement that runs longer than `timeout_ms` (`statement_timeout`; dropping
///   the future instead would leave the statement running), which also fails the read.
pub async fn read_participants(
    db: &mut toasty::Db,
    viewer: &AuthUser,
    root_id: &str,
    channel_id: &str,
    timeout_ms: u64,
) -> Result<Value, AppError> {
    let mut tx = db.transaction().await?;
    toasty::sql::statement(format!("SET LOCAL statement_timeout={timeout_ms}"))
        .exec(&mut tx)
        .await?;
    let actor = crate::apps::events::Actor {
        user: viewer.clone(),
        principal: None,
    };
    actor.channel(&mut tx, channel_id, false).await?;
    let participants =
        crate::apps::one(&mut tx, participants_sql(), &[root_id, channel_id]).await?;
    tx.commit().await?;
    Ok(participants)
}

async fn thread_operation(
    state: AppState,
    auth: AuthUser,
    id: String,
    operation: &str,
    with_participants: bool,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    uuid::Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid message ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth.id).await?;
    // A batch holds the canonical root before moving children. Acquire the same
    // lock before resolving again, so a concurrent leave cannot land on a child
    // whose membership has already been merged.
    if operation != "read" {
        crate::apps::query(
            &mut tx,
            "SELECT '{}'::text FROM messages WHERE id=thread_root($1) FOR NO KEY UPDATE",
            &[&id],
        )
        .await?;
    }
    let root = crate::apps::one(&mut tx,
        "SELECT json_build_object('id',r.id,'channel_id',r.channel_id)::text FROM messages requested JOIN messages r ON r.id=CASE WHEN (SELECT phase FROM thread_rollout WHERE id)='collecting' OR ((SELECT phase FROM thread_rollout WHERE id)='normalizing' AND NOT EXISTS(SELECT 1 FROM thread_migration_completed done WHERE done.thread_id=thread_root(requested.id))) THEN requested.id ELSE thread_root(requested.id) END WHERE requested.id=$1 AND requested.deleted_at IS NULL AND r.deleted_at IS NULL",
        &[&id]).await?;
    let root_id = root["id"].as_str().unwrap();
    let channel_id = root["channel_id"].as_str().unwrap();
    let actor = crate::apps::events::Actor {
        user: auth.clone(),
        principal: None,
    };
    let channel = actor
        .channel(
            &mut tx,
            channel_id,
            operation == "join" || operation == "unarchive",
        )
        .await?;
    if operation == "unarchive" {
        crate::apps::events::posting(&mut tx, &auth.id, channel_id, state.config.dev_auth_enabled)
            .await?;
    }
    // Same lock as the reply trigger and archive worker. Reading needs no write lock.
    let lock = if operation == "read" {
        ""
    } else {
        " FOR NO KEY UPDATE"
    };
    let status = crate::apps::one(&mut tx, &format!(
        "SELECT json_build_object('archived',thread_archive_at IS NOT NULL)::text FROM messages WHERE id=$1 AND deleted_at IS NULL{lock}"), &[root_id]).await?;
    let changed = match operation {
        "join" => {
            if status["archived"] == true {
                return Err(AppError::Conflict("Reopen the thread before joining".into()));
            }
            crate::apps::query(&mut tx,
                "INSERT INTO thread_participants(thread_id,user_id) VALUES($1,$2) ON CONFLICT(thread_id,user_id) DO UPDATE SET left_at=NULL,joined_at=now() AT TIME ZONE 'UTC',updated_at=now() AT TIME ZONE 'UTC' WHERE thread_participants.left_at IS NOT NULL RETURNING '{}'::text",
                &[root_id,&auth.id]).await?
        }
        // A member can leave a thread that has no row yet (the row records the leave). A reader who is
        // not a member (a public channel can be read without joining it) may only end a row it
        // already has: a new row would make the thread tracked, and the first reply of every member
        // would then need a join.
        "leave" if channel["member"] == true => crate::apps::query(&mut tx,
            "INSERT INTO thread_participants(thread_id,user_id,left_at) VALUES($1,$2,now() AT TIME ZONE 'UTC') ON CONFLICT(thread_id,user_id) DO UPDATE SET left_at=now() AT TIME ZONE 'UTC',updated_at=now() AT TIME ZONE 'UTC' WHERE thread_participants.left_at IS NULL RETURNING '{}'::text",
            &[root_id,&auth.id]).await?,
        "leave" => crate::apps::query(&mut tx,
            "UPDATE thread_participants SET left_at=now() AT TIME ZONE 'UTC',updated_at=now() AT TIME ZONE 'UTC' WHERE thread_id=$1 AND user_id=$2 AND left_at IS NULL RETURNING '{}'::text",
            &[root_id,&auth.id]).await?,
        "unarchive" => crate::apps::query(&mut tx,
            "UPDATE messages SET thread_archive_at=NULL,thread_activity_at=now() AT TIME ZONE 'UTC' WHERE id=$1 AND thread_archive_at IS NOT NULL RETURNING '{}'::text",
            &[root_id]).await?,
        _ => Vec::new(),
    };
    let mut row = crate::apps::one(&mut tx,
        "SELECT json_build_object('id',m.id,'channel_id',m.channel_id,'thread_archive_at',m.thread_archive_at,'membership',CASE WHEN p.user_id IS NULL THEN 'none' WHEN p.left_at IS NULL THEN 'joined' ELSE 'left' END)::text FROM messages m LEFT JOIN thread_participants p ON p.thread_id=m.id AND p.user_id=$2 WHERE m.id=$1",
        &[root_id,&auth.id]).await?;
    row["can_participate"] = json!(channel["member"] == true);
    // Who is in the thread, and whether replies need a join. Reading is already authorized by
    // the channel check above. P = participant rows of this one thread (left rows included).
    // - `keys`: the single primary-key range scan (thread_id,*) of those P rows. `tracked`,
    //   the count and the list are all derived from it; no second probe of the table. A separate
    //   `EXISTS`/`LIMIT 1` probe let the planner choose a sequential scan that skipped every
    //   other thread's rows before reaching the target.
    // - `users` is read through a LATERAL primary-key probe per live key (`OFFSET 0` keeps the
    //   planner from flattening it into a join), so it costs P probes whichever join method is
    //   chosen. A plain `JOIN keys` with `id = ANY(ARRAY(keys))` was seen to become a Nested Loop
    //   with a Join Filter on PostgreSQL 16 (P x P `users` rows), or a Hash Join over all users.
    //   The channel membership check is a scalar subquery, so it stays one primary-key probe per
    //   key as well (an `EXISTS` could be planned as a semi-join that scans the channel's members).
    // The list sorts the live rows (top-N, LIMIT 50): O(P log 50), independent of other threads.
    // A write holds the root lock (it serializes replies, joins and leaves of this thread) until
    // it commits, so its O(P) read runs after the commit, in its own transaction: the lock is held
    // for the write only, whatever P is. A read has no lock and stays in its transaction.
    let participants = if with_participants && operation == "read" {
        Some(crate::apps::one(&mut tx, participants_sql(), &[root_id, channel_id]).await?)
    } else {
        // `tracked` is always answered, one index entry, inside the transaction (after the write):
        // a write whose list is read afterwards, and fails or runs out of time, still says it.
        let tracked = crate::apps::one(&mut tx, thread_tracked_sql(), &[root_id]).await?;
        row["tracked"] = tracked["tracked"].clone();
        None
    };
    let txid = if changed.is_empty() {
        None
    } else {
        Some(get_current_txid(&mut tx).await?)
    };
    tx.commit().await?;
    let participants = match participants {
        Some(participants) => Some(participants),
        None if !with_participants => None,
        // The write is already committed. If only this read fails, answer without the list
        // (the client keeps what it has and refreshes) instead of reporting a failed write.
        None => match read_participants(
            &mut db,
            &auth,
            root_id,
            channel_id,
            state.config.thread_participants_read_timeout_ms,
        )
        .await
        {
            Ok(participants) => Some(participants),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    thread = root_id,
                    "reading participants after a write failed"
                );
                None
            }
        },
    };
    if let Some(participants) = participants {
        row["tracked"] = participants["tracked"].clone();
        row["participant_count"] = participants["count"].clone();
        row["participants"] = participants["items"].clone();
    }
    Ok(Json(ApiResponse::new(row, txid)))
}

pub async fn archive_idle_threads(db: &toasty::Db) -> Result<(), AppError> {
    let mut db = db.clone();
    toasty::sql::statement(
        "WITH idle AS ( \
           SELECT root.id FROM messages root \
           WHERE root.deleted_at IS NULL AND ((SELECT phase FROM thread_rollout WHERE id)<>'ready' OR root.parent_id IS NULL) \
             AND root.thread_archive_at IS NULL \
             AND EXISTS(SELECT 1 FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL) \
             AND root.thread_activity_at IS NOT NULL \
             AND root.thread_activity_at < (now() AT TIME ZONE 'UTC') - ($1::int * interval '1 day') \
           ORDER BY root.thread_activity_at,root.id LIMIT 1000 FOR UPDATE SKIP LOCKED \
         ) UPDATE messages root SET thread_archive_at=now() AT TIME ZONE 'UTC' FROM idle WHERE root.id=idle.id",
    )
    .bind(THREAD_IDLE_DAYS.to_string())
    .exec(&mut db)
    .await?;
    Ok(())
}

pub async fn worker(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticks.tick().await;
        if let Err(error) = archive_idle_threads(&state.db).await {
            tracing::warn!(error = %error, "Thread auto-archive worker failed");
        }
    }
}
