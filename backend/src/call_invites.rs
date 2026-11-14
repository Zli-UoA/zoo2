//! 通話の招待 (呼び出し)。仕組みは docs/calls.md。
//!
//! 招待は `call_invites` の行で、`pending` の間だけ呼び出し中。応答・拒否・取り消し・不在で終わる。
//! 呼ばれた人には Electric の shape（`/api/shapes/call_invites`）と Web Push で届ける。

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::AppError,
    models::{Channel, Notification, now_utc_millis},
    push::{self, CALL_INVITE_TTL_SECONDS, CallInvitePush},
    routes::calls::{MAX_CALL_PARTICIPANTS, authorize_call_access},
    state::AppState,
};

/// 1 回の招待で呼べる人数の上限（通話の定員から、呼ぶ本人を除いた数）。
const MAX_INVITEES_PER_REQUEST: usize = MAX_CALL_PARTICIPANTS - 1;

const RETURNING_INVITE: &str = "RETURNING json_build_object(
    'id', id, 'channel_id', channel_id, 'inviter_id', inviter_id, 'invitee_id', invitee_id)::text";

fn parse_rows<T: DeserializeOwned>(rows: Vec<toasty::stmt::Value>) -> Vec<T> {
    rows.into_iter()
        .filter_map(|row| {
            if let toasty::stmt::Value::Record(rec) = row
                && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
            {
                serde_json::from_str::<T>(&json).ok()
            } else {
                None
            }
        })
        .collect()
}

/// DM で、空の部屋に最初に入った人が、相手（DM の他のメンバー）を呼ぶ。すでに通話に入っている人は呼ばない。
/// グループ DM でも、通話の空きの数までしか呼ばない。
pub async fn create_dm_invites(
    db: &mut toasty::Db,
    channel_id: &str,
    inviter_id: &str,
) -> Result<Vec<CallInvitePush>, AppError> {
    let rows = toasty::sql::query(
        "SELECT user_id FROM channel_user WHERE channel_id = $1 AND user_id <> $2 ORDER BY user_id",
    )
    .bind(channel_id.to_string())
    .bind(inviter_id.to_string())
    .exec(&mut *db)
    .await?;
    let members: Vec<String> = rows
        .into_iter()
        .filter_map(|row| match row {
            toasty::stmt::Value::Record(rec) => match rec.fields.into_iter().next() {
                Some(toasty::stmt::Value::String(id)) => Some(id),
                _ => None,
            },
            _ => None,
        })
        .collect();
    if members.is_empty() {
        return Ok(Vec::new());
    }
    insert_invites(db, channel_id, inviter_id, &members).await
}

/// 招待を作る。空きの数（定員 - 在室 - 呼び出し中）までしか作らず、1 回の要求の中でも累積させる。
///
/// 空きの確認から挿入までを、チャンネル行のロック（通話への参加と同じ）の下で行い、同時の要求や
/// 入室と直列化する。そうしないと、別々の相手への 2 件の要求が同じ空きを読んで、定員を超える。
/// 呼べないユーザー（bot・無効化）、すでに入室している人、すでに呼び出し中の人は、数える前に除く。
async fn insert_invites(
    db: &mut toasty::Db,
    channel_id: &str,
    inviter_id: &str,
    invitee_ids: &[String],
) -> Result<Vec<CallInvitePush>, AppError> {
    let ids_json = serde_json::json!(invitee_ids).to_string();
    let mut tx = db.transaction().await?;
    toasty::sql::query("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel_id.to_string())
        .exec(&mut tx)
        .await?;
    let rows = toasty::sql::query(format!(
        "WITH slots AS (
             SELECT greatest(0, {MAX_CALL_PARTICIPANTS}
               - (SELECT count(*) FROM call_presence
                  WHERE channel_id = $1 AND expires_at > statement_timestamp())
               - (SELECT count(*) FROM call_invites
                  WHERE channel_id = $1 AND status = 'pending'))::int AS n
         ),
         candidates AS (
             SELECT u.id AS user_id, row_number() OVER (ORDER BY u.id) AS rn
             FROM users u
             WHERE u.id IN (SELECT jsonb_array_elements_text($3::jsonb))
               AND NOT u.is_bot AND NOT u.is_deactivated
               AND NOT EXISTS (
                   SELECT 1 FROM call_presence p
                   WHERE p.channel_id = $1 AND p.user_id = u.id
                     AND p.expires_at > statement_timestamp())
               AND NOT EXISTS (
                   SELECT 1 FROM call_invites i
                   WHERE i.channel_id = $1 AND i.invitee_id = u.id AND i.status = 'pending')
         )
         INSERT INTO call_invites (channel_id, inviter_id, invitee_id, expires_at)
         SELECT $1::text, $2::text, candidates.user_id,
                statement_timestamp() + interval '{CALL_INVITE_TTL_SECONDS} seconds'
         FROM candidates, slots WHERE candidates.rn <= slots.n
         ON CONFLICT (channel_id, invitee_id) WHERE status = 'pending' DO NOTHING
         {RETURNING_INVITE}"
    ))
    .bind(channel_id.to_string())
    .bind(inviter_id.to_string())
    .bind(ids_json)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(parse_rows(rows))
}

/// 呼ばれた人が通話に入ったら、その人の `pending` を `accepted` にする。終わった招待を返す。
pub async fn accept_on_join(
    db: &mut toasty::Db,
    channel_id: &str,
    user_id: &str,
) -> Result<Vec<CallInvitePush>, AppError> {
    let rows = toasty::sql::query(format!(
        "UPDATE call_invites SET status = 'accepted', responded_at = statement_timestamp()
         WHERE channel_id = $1 AND invitee_id = $2 AND status = 'pending'
         {RETURNING_INVITE}"
    ))
    .bind(channel_id.to_string())
    .bind(user_id.to_string())
    .exec(db)
    .await?;
    Ok(parse_rows(rows))
}

/// 部屋が空になった（呼んだ人が全員抜けた）のに `pending` のままの招待を `cancelled` にする。
/// `channel_id` が `None` なら全チャンネルが対象（定期処理）。失敗しても通話の処理は止めない。
pub async fn cancel_orphaned(db: &mut toasty::Db, channel_id: &str) {
    cancel_orphaned_in(db, Some(channel_id)).await;
}

async fn cancel_orphaned_in(db: &mut toasty::Db, channel_id: Option<&str>) {
    let result = toasty::sql::query(format!(
        "UPDATE call_invites c SET status = 'cancelled', responded_at = statement_timestamp()
         WHERE c.status = 'pending'
           AND ($1::text IS NULL OR c.channel_id = $1)
           AND NOT EXISTS (
             SELECT 1 FROM call_presence p
             WHERE p.channel_id = c.channel_id AND p.expires_at > statement_timestamp())
         {RETURNING_INVITE}"
    ))
    .bind(channel_id.map(ToString::to_string))
    .exec(db)
    .await;
    match result {
        Ok(rows) => push::dispatch_call_invite_ends(db.clone(), parse_rows(rows)),
        Err(e) => tracing::warn!("Failed to cancel orphaned call invites: {e}"),
    }
}

#[derive(Debug, Deserialize)]
struct MissedRow {
    id: String,
    channel_id: String,
    inviter_id: String,
    invitee_id: String,
    notification_id: String,
    /// 通知を作ったか（チャンネルが削除済みなどで作らなかった招待には、push も送らない）。
    notified: bool,
}

/// 定期処理。期限を過ぎた `pending` を `missed` にして不在着信の通知を作り、空の部屋の招待を取り消す。
///
/// 複数のレプリカが同時に走っても、`FOR UPDATE SKIP LOCKED` で同じ招待を二重に処理しない。
/// 通知の id は招待から決まる値なので、やり直しても重複しない。
pub async fn sweep(db: &mut toasty::Db) -> Result<(), AppError> {
    // 参加と呼び出しが同時に起きて、入室済みの人に `pending` が残ったものを応答済みにする。
    let rows = toasty::sql::query(format!(
        "UPDATE call_invites c SET status = 'accepted', responded_at = statement_timestamp()
         WHERE c.status = 'pending' AND EXISTS (
             SELECT 1 FROM call_presence p
             WHERE p.channel_id = c.channel_id AND p.user_id = c.invitee_id
               AND p.expires_at > statement_timestamp())
         {RETURNING_INVITE}"
    ))
    .exec(db)
    .await?;
    push::dispatch_call_invite_ends(db.clone(), parse_rows(rows));
    let rows = toasty::sql::query(
        "WITH due AS (
             SELECT i.id FROM call_invites i
             WHERE i.status = 'pending' AND i.expires_at <= statement_timestamp()
               AND NOT EXISTS (
                   SELECT 1 FROM call_presence p
                   WHERE p.channel_id = i.channel_id AND p.user_id = i.invitee_id
                     AND p.expires_at > statement_timestamp())
             ORDER BY i.expires_at LIMIT 100 FOR UPDATE OF i SKIP LOCKED
         ),
         missed AS (
             UPDATE call_invites c SET status = 'missed', responded_at = statement_timestamp()
             FROM due WHERE c.id = due.id
             RETURNING c.id, c.channel_id, c.inviter_id, c.invitee_id
         ),
         notified AS (
             INSERT INTO notification_events (id, user_id, actor_id, channel_id, notification_type, source)
             SELECT md5('missed_call:' || m.id::text)::uuid::text, m.invitee_id, m.inviter_id,
                    m.channel_id, 'missed_call', 'channel'
             FROM missed m JOIN channels ch ON ch.id = m.channel_id AND ch.deleted_at IS NULL
             ON CONFLICT (id) DO NOTHING
             RETURNING id
         )
         SELECT json_build_object(
             'id', m.id, 'channel_id', m.channel_id, 'inviter_id', m.inviter_id,
             'invitee_id', m.invitee_id,
             'notification_id', md5('missed_call:' || m.id::text)::uuid::text,
             'notified', EXISTS (
                 SELECT 1 FROM notified n WHERE n.id = md5('missed_call:' || m.id::text)::uuid::text))::text
         FROM missed m",
    )
    .exec(db)
    .await?;
    let missed: Vec<MissedRow> = parse_rows(rows);
    if !missed.is_empty() {
        let now = now_utc_millis();
        let notifications = missed
            .iter()
            .filter(|m| m.notified)
            .map(|m| Notification {
                id: m.notification_id.clone(),
                user_id: m.invitee_id.clone(),
                actor_id: m.inviter_id.clone(),
                channel_id: m.channel_id.clone(),
                message_id: None,
                notification_type: "missed_call".into(),
                is_read: false,
                created_at: now,
            })
            .collect();
        push::dispatch_call_invite_ends(
            db.clone(),
            missed
                .iter()
                .map(|m| CallInvitePush {
                    id: m.id.clone(),
                    channel_id: m.channel_id.clone(),
                    inviter_id: m.inviter_id.clone(),
                    invitee_id: m.invitee_id.clone(),
                })
                .collect(),
        );
        push::dispatch_db(db.clone(), notifications);
    }
    cancel_orphaned_in(db, None).await;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateInvitesRequest {
    pub user_ids: Vec<String>,
}

/// 招待される人が、そのチャンネルの通話に入れる人か（有効なユーザーで、非公開・guest ならメンバー）。
async fn invitee_can_join(
    db: &mut toasty::Db,
    channel: &Channel,
    user_id: &str,
) -> Result<bool, AppError> {
    let usable = toasty::sql::query(
        "SELECT 1 FROM users WHERE id = $1 AND NOT is_bot AND NOT is_deactivated",
    )
    .bind(user_id.to_string())
    .exec(&mut *db)
    .await?;
    if usable.is_empty() {
        return Ok(false);
    }
    let is_guest = crate::apps::events::guest_in_db(&mut *db, user_id).await?;
    if channel.is_private || is_guest {
        let member =
            toasty::sql::query("SELECT 1 FROM channel_user WHERE channel_id = $1 AND user_id = $2")
                .bind(channel.id.clone())
                .bind(user_id.to_string())
                .exec(&mut *db)
                .await?;
        return Ok(!member.is_empty());
    }
    Ok(true)
}

/// `POST /api/channels/{id}/call/invites`: 通話中の人が、チャンネルのメンバーを呼ぶ。DM では使えない
/// （DM は、通話を始めると相手を自動で呼ぶ）。
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<CreateInvitesRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut db = state.db.clone();
    let channel = authorize_call_access(&mut db, &channel_id, &user).await?;
    if channel.is_dm {
        return Err(AppError::BadRequest(
            "DM では、通話を始めると相手を自動で呼びます".into(),
        ));
    }

    let mut ids: Vec<String> = Vec::new();
    for id in body.user_ids {
        if id != user.id && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(AppError::BadRequest("user_ids is empty".into()));
    }
    if ids.len() > MAX_INVITEES_PER_REQUEST {
        return Err(AppError::BadRequest(format!(
            "Too many users (max {MAX_INVITEES_PER_REQUEST})"
        )));
    }

    let in_call = toasty::sql::query(
        "SELECT 1 FROM call_presence
         WHERE channel_id = $1 AND user_id = $2 AND expires_at > statement_timestamp()",
    )
    .bind(channel_id.clone())
    .bind(user.id.clone())
    .exec(&mut db)
    .await?;
    if in_call.is_empty() {
        return Err(AppError::Forbidden(
            "You must join the call before inviting others".into(),
        ));
    }

    // 見えないチャンネルのメンバーかどうかを漏らさないよう、どれが理由でも同じ 404。
    for id in &ids {
        if !invitee_can_join(&mut db, &channel, id).await? {
            return Err(AppError::NotFound("User not found in this channel".into()));
        }
    }

    let created = insert_invites(&mut db, &channel_id, &user.id, &ids).await?;
    push::dispatch_call_invites(state.db.clone(), created.clone());

    Ok(Json(serde_json::json!({
        "invites": created
            .iter()
            .map(|i| serde_json::json!({ "id": i.id, "invitee_id": i.invitee_id }))
            .collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
struct InviteRow {
    channel_id: String,
    inviter_id: String,
    invitee_id: String,
    status: String,
    expired: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Accept,
    Decline,
    Cancel,
}

impl Action {
    fn status(self) -> &'static str {
        match self {
            Self::Accept => "accepted",
            Self::Decline => "declined",
            Self::Cancel => "cancelled",
        }
    }
}

async fn respond(
    state: AppState,
    user: AuthUser,
    id: String,
    action: Action,
) -> Result<Json<serde_json::Value>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("invite id is not a valid UUID: {id}")))?;
    let mut db = state.db.clone();
    let rows = toasty::sql::query(
        "SELECT json_build_object(
             'channel_id', channel_id, 'inviter_id', inviter_id, 'invitee_id', invitee_id,
             'status', status, 'expired', expires_at <= statement_timestamp())::text
         FROM call_invites WHERE id = $1::uuid",
    )
    .bind(id.clone())
    .exec(&mut db)
    .await?;
    let row: InviteRow = parse_rows(rows)
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("Invite not found".into()))?;

    // 応答は呼ばれた本人だけ、取り消しは呼んだ本人だけ。関係のない人には存在を見せない。
    let allowed = match action {
        Action::Accept | Action::Decline => row.invitee_id == user.id,
        Action::Cancel => row.inviter_id == user.id,
    };
    if !allowed {
        return Err(AppError::NotFound("Invite not found".into()));
    }
    if row.status != "pending" || (action == Action::Accept && row.expired) {
        return Err(AppError::Conflict("Invite is no longer pending".into()));
    }
    if action == Action::Accept {
        // 脱退や非公開化で、もう入れないチャンネルの通話には応答させない。
        authorize_call_access(&mut db, &row.channel_id, &user).await?;
    }

    let updated = toasty::sql::query(format!(
        "UPDATE call_invites SET status = $2, responded_at = statement_timestamp()
         WHERE id = $1::uuid AND status = 'pending'
           AND ($3 = false OR expires_at > statement_timestamp())
         {RETURNING_INVITE}"
    ))
    .bind(id)
    .bind(action.status().to_string())
    .bind(action == Action::Accept)
    .exec(&mut db)
    .await?;
    let ended: Vec<CallInvitePush> = parse_rows(updated);
    if ended.is_empty() {
        return Err(AppError::Conflict("Invite is no longer pending".into()));
    }
    // 応答・拒否・取り消しを、呼ばれた人の他の端末に伝えて、出ている通知を閉じる。
    push::dispatch_call_invite_ends(state.db.clone(), ended);

    Ok(Json(
        serde_json::json!({ "ok": true, "channel_id": row.channel_id }),
    ))
}

/// `POST /api/call-invites/{id}/accept`: 呼ばれた人が応答する（通話への参加は、続けて SSE で行う）。
pub async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    respond(state, user, id, Action::Accept).await
}

/// `POST /api/call-invites/{id}/decline`: 呼ばれた人が拒否する。
pub async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    respond(state, user, id, Action::Decline).await
}

/// `POST /api/call-invites/{id}/cancel`: 呼んだ人が呼び出しをやめる。
pub async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    respond(state, user, id, Action::Cancel).await
}
