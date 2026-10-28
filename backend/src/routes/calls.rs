use std::{
    collections::HashMap,
    convert::Infallible,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::AppError,
    models::{Channel, ChannelUser},
    state::AppState,
};

pub const MAX_CALL_PARTICIPANTS: usize = 6;
/// PostgreSQL NOTIFY ペイロード上限 (8000 bytes) を超えない安全な上限
pub const MAX_SIGNAL_PAYLOAD_SIZE: usize = 7000;
pub const CALL_PRESENCE_TTL_SECONDS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallParticipant {
    pub session_id: Uuid,
    pub user_id: String,
    pub user_name: String,
    pub display_name: String,
    pub is_muted: bool,
    pub joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicCallParticipant {
    pub user_id: String,
    pub user_name: String,
    pub display_name: String,
    pub is_muted: bool,
    pub joined_at: String,
}

impl From<&CallParticipant> for PublicCallParticipant {
    fn from(p: &CallParticipant) -> Self {
        Self {
            user_id: p.user_id.clone(),
            user_name: p.user_name.clone(),
            display_name: p.display_name.clone(),
            is_muted: p.is_muted,
            joined_at: p.joined_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CallStatusResponse {
    pub is_active: bool,
    pub channel_id: String,
    pub participant_count: usize,
    pub participants: Vec<PublicCallParticipant>,
    pub max_participants: usize,
    pub ice_servers: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveCallSummary {
    pub channel_id: String,
    pub participant_count: usize,
    pub participants: Vec<PublicCallParticipant>,
}

#[derive(Debug, Deserialize)]
pub struct SignalPayload {
    pub session_id: Uuid,
    pub target_session_id: Uuid,
    pub signal: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatePayload {
    pub session_id: Uuid,
    pub is_muted: bool,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatPayload {
    pub session_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct LeavePayload {
    pub session_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    pub session_id: Uuid,
}

#[derive(Debug, Clone)]
pub enum CallServerEvent {
    RoomState {
        participants: Vec<CallParticipant>,
        max_participants: usize,
        ice_servers: serde_json::Value,
    },
    ParticipantJoined {
        participant: CallParticipant,
    },
    ParticipantLeft {
        session_id: Uuid,
        user_id: String,
    },
    ParticipantUpdated {
        participant: CallParticipant,
    },
    Signal {
        from_session_id: Uuid,
        signal: serde_json::Value,
    },
}

impl CallServerEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::RoomState { .. } => "room_state",
            Self::ParticipantJoined { .. } => "participant_joined",
            Self::ParticipantLeft { .. } => "participant_left",
            Self::ParticipantUpdated { .. } => "participant_updated",
            Self::Signal { .. } => "signal",
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::RoomState {
                participants,
                max_participants,
                ice_servers,
            } => serde_json::json!({
                "participants": participants,
                "max_participants": max_participants,
                "ice_servers": ice_servers,
            }),
            Self::ParticipantJoined { participant } => serde_json::json!({
                "participant": participant,
            }),
            Self::ParticipantLeft {
                session_id,
                user_id,
            } => serde_json::json!({
                "session_id": session_id,
                "user_id": user_id,
            }),
            Self::ParticipantUpdated { participant } => serde_json::json!({
                "participant": participant,
            }),
            Self::Signal {
                from_session_id,
                signal,
            } => serde_json::json!({
                "from_session_id": from_session_id,
                "signal": signal,
            }),
        }
    }
}

/// PostgreSQL LISTEN/NOTIFY による全レプリカ間ブロードキャスト用イベント
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CallDistributedEvent {
    #[serde(rename = "signal")]
    Signal {
        channel_id: String,
        target_session_id: Uuid,
        from_session_id: Uuid,
        signal: serde_json::Value,
    },
    #[serde(rename = "participant_joined")]
    ParticipantJoined {
        channel_id: String,
        participant: CallParticipant,
    },
    #[serde(rename = "participant_left")]
    ParticipantLeft {
        channel_id: String,
        session_id: Uuid,
        user_id: String,
    },
    #[serde(rename = "participant_updated")]
    ParticipantUpdated {
        channel_id: String,
        participant: CallParticipant,
    },
    #[serde(rename = "room_closed")]
    RoomClosed {
        channel_id: String,
    },
}

struct LocalSession {
    channel_id: String,
    #[allow(dead_code)]
    user_id: String,
    sender: mpsc::Sender<CallServerEvent>,
}

#[derive(Clone, Default)]
pub struct CallRegistry {
    local_sessions: Arc<RwLock<HashMap<Uuid, LocalSession>>>,
    db: Arc<RwLock<Option<toasty::Db>>>,
}

impl CallRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_db(&self, db: toasty::Db) {
        *self.db.write().await = Some(db);
    }

    pub async fn get_db(&self) -> Option<toasty::Db> {
        self.db.read().await.clone()
    }

    /// PostgreSQL の LISTEN zoo_call_events を別タスクで実行し、
    /// 受信した通知をローカルセッションへディスパッチする。
    pub fn start_listener(&self, database_url: String) {
        let registry = self.clone();
        tokio::spawn(async move {
            loop {
                tracing::debug!("Connecting to PostgreSQL for call signaling LISTEN...");
                match tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await {
                    Ok((client, mut connection)) => {
                        let mut stream =
                            futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));
                        let listen_fut = client.batch_execute("LISTEN zoo_call_events");
                        tokio::pin!(listen_fut);

                        let mut listen_active = true;
                        tracing::info!("Subscribed to PostgreSQL channel zoo_call_events");

                        loop {
                            use futures_util::StreamExt;
                            tokio::select! {
                                res = &mut listen_fut, if listen_active => {
                                    listen_active = false;
                                    if let Err(e) = res {
                                        tracing::warn!("Failed to execute LISTEN zoo_call_events: {e}");
                                        break;
                                    }
                                }
                                item = stream.next() => {
                                    match item {
                                        Some(Ok(tokio_postgres::AsyncMessage::Notification(notification))) => {
                                            if notification.channel() == "zoo_call_events" {
                                                registry.dispatch_distributed_event(notification.payload()).await;
                                            }
                                        }
                                        Some(Ok(_)) => {}
                                        Some(Err(e)) => {
                                            tracing::warn!("PostgreSQL notification stream error: {e}");
                                            break;
                                        }
                                        None => {
                                            tracing::warn!("PostgreSQL notification connection ended");
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to connect to PostgreSQL for LISTEN: {e}");
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    /// 期限切れセッションの定期スイープタスクを起動
    pub fn start_sweeper(&self, mut db: toasty::Db) {
        let registry = self.clone();
        let db_for_set = db.clone();
        tokio::spawn(async move {
            registry.set_db(db_for_set).await;
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Err(e) = Self::sweep_stale_presence(&mut db).await {
                    tracing::warn!("Failed to sweep stale call presence: {e}");
                }
            }
        });
    }

    /// DB 上の期限切れセッションを検出し削除・通知する
    pub async fn sweep_stale_presence(db: &mut toasty::Db) -> Result<(), AppError> {
        let rows = toasty::sql::query(
            "DELETE FROM call_presence
             WHERE ctid IN (
                 SELECT ctid FROM call_presence
                 WHERE expires_at <= statement_timestamp()
                 LIMIT 100 FOR UPDATE SKIP LOCKED
             )
             RETURNING json_build_object(
                 'channel_id', channel_id,
                 'session_id', session_id,
                 'user_id', user_id
             )::text",
        )
        .exec(db)
        .await?;

        #[derive(Deserialize)]
        struct ExpiredRow {
            channel_id: String,
            session_id: Uuid,
            user_id: String,
        }

        for row in rows {
            if let toasty::stmt::Value::Record(rec) = row
                && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
                && let Ok(expired) = serde_json::from_str::<ExpiredRow>(&json)
            {
                let event = CallDistributedEvent::ParticipantLeft {
                    channel_id: expired.channel_id,
                    session_id: expired.session_id,
                    user_id: expired.user_id,
                };
                let _ = Self::notify_event(db, &event).await;
            }
        }
        Ok(())
    }

    /// 受信した分散イベントを、このインスタンス内のローカル SSE クライアントへ配送する
    pub async fn dispatch_distributed_event(&self, payload: &str) {
        let event: CallDistributedEvent = match serde_json::from_str(payload) {
            Ok(ev) => ev,
            Err(e) => {
                tracing::warn!("Failed to parse distributed call event: {e}, payload: {payload}");
                return;
            }
        };

        match event {
            CallDistributedEvent::Signal {
                channel_id,
                target_session_id,
                from_session_id,
                signal,
            } => {
                let sessions = self.local_sessions.read().await;
                if let Some(target) = sessions.get(&target_session_id) {
                    // チャンネル ID が一致していることを必ず照合（[P1] 越境配送防止）
                    if target.channel_id != channel_id {
                        tracing::warn!(
                            target_session_id = %target_session_id,
                            event_channel = %channel_id,
                            session_channel = %target.channel_id,
                            "Signal channel mismatch: ignoring signal"
                        );
                        return;
                    }

                    let sse_event = CallServerEvent::Signal {
                        from_session_id,
                        signal,
                    };
                    if let Err(e) = target.sender.try_send(sse_event) {
                        tracing::warn!(
                            target_session_id = %target_session_id,
                            channel_id = %channel_id,
                            "Failed to deliver signal to local session (queue full or closed): {e}. Terminating stale session."
                        );
                        // 宛先セッションのバッファ満杯時は不通セッションを切断・強制退出（[P2]）
                        let dropped_user_id = target.user_id.clone();
                        drop(sessions);

                        let mut write_sessions = self.local_sessions.write().await;
                        write_sessions.remove(&target_session_id);
                        drop(write_sessions);

                        if let Some(mut db) = self.get_db().await {
                            let _ = toasty::sql::statement(
                                "DELETE FROM call_presence WHERE channel_id = $1 AND session_id = $2::uuid",
                            )
                            .bind(channel_id.clone())
                            .bind(target_session_id.to_string())
                            .exec(&mut db)
                            .await;

                            let left_event = CallDistributedEvent::ParticipantLeft {
                                channel_id,
                                session_id: target_session_id,
                                user_id: dropped_user_id,
                            };
                            let _ = Self::notify_event(&db, &left_event).await;
                        }
                    }
                }
            }
            CallDistributedEvent::ParticipantJoined {
                channel_id,
                participant,
            } => {
                let sessions = self.local_sessions.read().await;
                let sse_event = CallServerEvent::ParticipantJoined {
                    participant: participant.clone(),
                };
                for (sid, session) in sessions.iter() {
                    if session.channel_id == channel_id && *sid != participant.session_id {
                        let _ = session.sender.try_send(sse_event.clone());
                    }
                }
            }
            CallDistributedEvent::ParticipantLeft {
                channel_id,
                session_id,
                user_id,
            } => {
                let sse_event = CallServerEvent::ParticipantLeft {
                    session_id,
                    user_id,
                };
                let mut sessions = self.local_sessions.write().await;
                for (sid, session) in sessions.iter() {
                    if session.channel_id == channel_id && *sid != session_id {
                        let _ = session.sender.try_send(sse_event.clone());
                    }
                }
                sessions.remove(&session_id);
            }
            CallDistributedEvent::ParticipantUpdated {
                channel_id,
                participant,
            } => {
                let sessions = self.local_sessions.read().await;
                let sse_event = CallServerEvent::ParticipantUpdated {
                    participant: participant.clone(),
                };
                for (sid, session) in sessions.iter() {
                    if session.channel_id == channel_id && *sid != participant.session_id {
                        let _ = session.sender.try_send(sse_event.clone());
                    }
                }
            }
            CallDistributedEvent::RoomClosed { channel_id } => {
                let mut sessions = self.local_sessions.write().await;
                sessions.retain(|_, s| s.channel_id != channel_id);
            }
        }
    }

    /// ローカル宛先セッションのバッファが満杯かどうかを確認する
    pub async fn is_target_full(&self, session_id: &Uuid) -> bool {
        let sessions = self.local_sessions.read().await;
        if let Some(target) = sessions.get(session_id) {
            target.sender.capacity() == 0
        } else {
            false
        }
    }

    /// ローカルセッションを登録
    pub async fn register_local_session(
        &self,
        session_id: Uuid,
        channel_id: String,
        user_id: String,
        sender: mpsc::Sender<CallServerEvent>,
    ) {
        let mut sessions = self.local_sessions.write().await;
        sessions.insert(
            session_id,
            LocalSession {
                channel_id,
                user_id,
                sender,
            },
        );
    }

    /// ローカルセッションを解除
    pub async fn unregister_local_session(&self, session_id: &Uuid) {
        let mut sessions = self.local_sessions.write().await;
        sessions.remove(session_id);
    }

    /// PostgreSQL の pg_notify を発行して全インスタンスへイベントをブロードキャストする
    pub async fn notify_event(
        db: &toasty::Db,
        event: &CallDistributedEvent,
    ) -> Result<(), AppError> {
        let mut db = db.clone();
        let payload = serde_json::to_string(event).map_err(|e| AppError::Internal(e.into()))?;
        toasty::sql::statement("SELECT pg_notify('zoo_call_events', $1)")
            .bind(payload)
            .exec(&mut db)
            .await
            .map_err(AppError::from)?;
        Ok(())
    }

    /// チャンネル削除時にルームを破棄する
    pub async fn close_room(&self, db: &toasty::Db, channel_id: &str) {
        let event = CallDistributedEvent::RoomClosed {
            channel_id: channel_id.to_string(),
        };
        let _ = Self::notify_event(db, &event).await;
        let mut sessions = self.local_sessions.write().await;
        sessions.retain(|_, s| s.channel_id != channel_id);
    }

    /// チャンネルからユーザーが脱退した際のクリーンアップ
    pub async fn remove_user_from_channel(
        &self,
        db: &toasty::Db,
        channel_id: &str,
        user_id: &str,
    ) {
        let mut db = db.clone();
        let rows = match toasty::sql::query(
            "DELETE FROM call_presence
             WHERE channel_id = $1 AND user_id = $2
             RETURNING json_build_object('session_id', session_id)::text",
        )
        .bind(channel_id.to_string())
        .bind(user_id.to_string())
        .exec(&mut db)
        .await
        {
            Ok(r) => r,
            Err(_) => return,
        };

        #[derive(Deserialize)]
        struct RemovedRow {
            session_id: Uuid,
        }

        for row in rows {
            if let toasty::stmt::Value::Record(rec) = row
                && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
                && let Ok(removed) = serde_json::from_str::<RemovedRow>(&json)
            {
                let event = CallDistributedEvent::ParticipantLeft {
                    channel_id: channel_id.to_string(),
                    session_id: removed.session_id,
                    user_id: user_id.to_string(),
                };
                let _ = Self::notify_event(&db, &event).await;
                self.unregister_local_session(&removed.session_id).await;
            }
        }
    }
}

pub struct SessionGuard {
    call_registry: CallRegistry,
    channel_id: String,
    session_id: Uuid,
    user_id: String,
    db: toasty::Db,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let registry = self.call_registry.clone();
        let channel_id = self.channel_id.clone();
        let session_id = self.session_id;
        let user_id = self.user_id.clone();
        let mut db = self.db.clone();

        tokio::spawn(async move {
            registry.unregister_local_session(&session_id).await;
            let del_res = toasty::sql::statement(
                "DELETE FROM call_presence WHERE channel_id = $1 AND session_id = $2::uuid",
            )
            .bind(channel_id.clone())
            .bind(session_id.to_string())
            .exec(&mut db)
            .await;

            if del_res.is_ok() {
                let event = CallDistributedEvent::ParticipantLeft {
                    channel_id,
                    session_id,
                    user_id,
                };
                let _ = CallRegistry::notify_event(&mut db, &event).await;
            }
        });
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{id}/call", get(get_call_status))
        .route("/api/channels/{id}/call/stream", get(call_stream))
        .route("/api/channels/{id}/call/signal", post(send_signal))
        .route("/api/channels/{id}/call/state", post(update_state))
        .route("/api/channels/{id}/call/heartbeat", post(heartbeat))
        .route("/api/channels/{id}/call/leave", post(leave_call))
        .route("/api/calls/active", get(list_active_calls))
}

async fn authorize_call_access(
    db: &mut toasty::Db,
    channel_id: &str,
    user: &AuthUser,
) -> Result<Channel, AppError> {
    let channel = Channel::get_by_id(&mut *db, channel_id)
        .await
        .map_err(|e| {
            if e.is_record_not_found() {
                AppError::NotFound("Channel not found".into())
            } else {
                AppError::from(e)
            }
        })?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound("Channel not found".into()));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::Forbidden("Channel is archived".into()));
    }

    if channel.is_private {
        let member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(channel_id)
                .and(ChannelUser::fields().user_id().eq(user.id.as_str())),
        )
        .first()
        .exec(&mut *db)
        .await?;

        if member.is_none() {
            return Err(AppError::NotFound("Channel not found".into()));
        }
    }

    Ok(channel)
}

async fn get_call_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<CallStatusResponse>, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;

    let ice_servers: serde_json::Value =
        serde_json::from_str(&state.config.webrtc_ice_servers_json)
            .unwrap_or_else(|_| serde_json::json!([{"urls": ["stun:stun.l.google.com:19302"]}]));

    let rows = toasty::sql::query(
        "SELECT json_build_object(
            'user_id', user_id,
            'user_name', user_name,
            'display_name', display_name,
            'is_muted', is_muted,
            'joined_at', joined_at
         )::text
         FROM call_presence
         WHERE channel_id = $1 AND expires_at > statement_timestamp()
         ORDER BY joined_at ASC",
    )
    .bind(channel_id.clone())
    .exec(&mut db)
    .await?;

    let mut participants = Vec::new();
    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
            && let Ok(participant) = serde_json::from_str::<PublicCallParticipant>(&json)
        {
            participants.push(participant);
        }
    }

    let is_active = !participants.is_empty();
    let participant_count = participants.len();

    Ok(Json(CallStatusResponse {
        is_active,
        channel_id,
        participant_count,
        participants,
        max_participants: MAX_CALL_PARTICIPANTS,
        ice_servers,
    }))
}

async fn call_stream(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;

    // 1. 期限切れレコードのクリーンアップ（通知付きで安全に実行）
    let _ = CallRegistry::sweep_stale_presence(&mut db).await;

    // 2. トランザクションを開始し、チャンネル行を排他ロックして同時参加を直列化 ([P2])
    let mut tx = db.transaction().await?;
    let lock_rows = toasty::sql::query("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel_id.clone())
        .exec(&mut tx)
        .await?;
    if lock_rows.is_empty() {
        return Err(AppError::NotFound("Channel not found".into()));
    }

    // ロック下で現在の参加者数をチェック
    let count_rows = toasty::sql::query(
        "SELECT count(*)::text
         FROM call_presence
         WHERE channel_id = $1 AND expires_at > statement_timestamp()",
    )
    .bind(channel_id.clone())
    .exec(&mut tx)
    .await?;

    let current_count: usize = count_rows
        .into_iter()
        .find_map(|row| {
            if let toasty::stmt::Value::Record(rec) = row
                && let Some(toasty::stmt::Value::String(cnt)) = rec.fields.into_iter().next()
            {
                cnt.parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    if current_count >= MAX_CALL_PARTICIPANTS {
        return Err(AppError::Conflict(format!(
            "Call room is full (max {MAX_CALL_PARTICIPANTS} participants)"
        )));
    }

    let now_str = crate::models::now_utc_millis().to_string();
    let session_str = query.session_id.to_string();

    // 3. call_presence に挿入
    let insert_res = toasty::sql::statement(
        "INSERT INTO call_presence (
            channel_id, session_id, user_id, user_name, display_name, is_muted, joined_at, expires_at
         ) VALUES ($1, $2::uuid, $3, $4, $5, false, $6, statement_timestamp() + interval '30 seconds')",
    )
    .bind(channel_id.clone())
    .bind(session_str)
    .bind(user.id.clone())
    .bind(user.user_name.clone())
    .bind(user.display_name.clone())
    .bind(now_str.clone())
    .exec(&mut tx)
    .await;

    if let Err(e) = insert_res {
        if AppError::is_unique_violation(&e, None) {
            return Err(AppError::Conflict("Session ID is already in use".into()));
        }
        return Err(AppError::from(e));
    }

    // 4. 自身を含む現在の有効参加者一覧を取得
    let rows = toasty::sql::query(
        "SELECT json_build_object(
            'session_id', session_id,
            'user_id', user_id,
            'user_name', user_name,
            'display_name', display_name,
            'is_muted', is_muted,
            'joined_at', joined_at
         )::text
         FROM call_presence
         WHERE channel_id = $1 AND expires_at > statement_timestamp()
         ORDER BY joined_at ASC",
    )
    .bind(channel_id.clone())
    .exec(&mut tx)
    .await?;

    let mut participants = Vec::new();
    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
            && let Ok(participant) = serde_json::from_str::<CallParticipant>(&json)
        {
            participants.push(participant);
        }
    }

    tx.commit().await?;

    let participant = CallParticipant {
        session_id: query.session_id,
        user_id: user.id.clone(),
        user_name: user.user_name.clone(),
        display_name: user.display_name.clone(),
        is_muted: false,
        joined_at: now_str,
    };

    // 5. ローカルキュー (容量 256) を作成し登録
    let (tx, rx) = mpsc::channel(256);
    state
        .call_registry
        .register_local_session(
            query.session_id,
            channel_id.clone(),
            user.id.clone(),
            tx.clone(),
        )
        .await;

    let ice_servers: serde_json::Value =
        serde_json::from_str(&state.config.webrtc_ice_servers_json)
            .unwrap_or_else(|_| serde_json::json!([{"urls": ["stun:stun.l.google.com:19302"]}]));

    // 6. 新規参加者自身への初期 RoomState イベント送信
    let room_state_event = CallServerEvent::RoomState {
        participants,
        max_participants: MAX_CALL_PARTICIPANTS,
        ice_servers,
    };
    let _ = tx.try_send(room_state_event);

    // 7. 全レプリカの他参加者へ ParticipantJoined イベントをブロードキャスト
    let join_event = CallDistributedEvent::ParticipantJoined {
        channel_id: channel_id.clone(),
        participant,
    };
    let _ = CallRegistry::notify_event(&mut db, &join_event).await;

    let guard = SessionGuard {
        call_registry: state.call_registry.clone(),
        channel_id: channel_id.clone(),
        session_id: query.session_id,
        user_id: user.id.clone(),
        db: state.db.clone(),
    };

    let stream = futures_util::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        match rx.recv().await {
            Some(msg) => {
                let event = Event::default()
                    .event(msg.event_type())
                    .data(msg.to_json().to_string());
                Some((Ok::<_, Infallible>(event), (rx, guard)))
            }
            None => None,
        }
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

async fn send_signal(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<SignalPayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;

    let payload_len = serde_json::to_string(&body.signal)
        .unwrap_or_default()
        .len();
    if payload_len > MAX_SIGNAL_PAYLOAD_SIZE {
        return Err(AppError::BadRequest("Signal payload too large".into()));
    }

    // 送信元と宛先セッションの存在および認証ユーザーとの整合性を検証
    let rows = toasty::sql::query(
        "SELECT json_build_object(
            'session_id', session_id,
            'user_id', user_id
         )::text
         FROM call_presence
         WHERE channel_id = $1
           AND session_id IN ($2::uuid, $3::uuid)
           AND expires_at > statement_timestamp()",
    )
    .bind(channel_id.clone())
    .bind(body.session_id.to_string())
    .bind(body.target_session_id.to_string())
    .exec(&mut db)
    .await?;

    #[derive(Deserialize)]
    struct PresenceRow {
        session_id: Uuid,
        user_id: String,
    }

    let mut found_sender = false;
    let mut found_target = false;

    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
            && let Ok(p) = serde_json::from_str::<PresenceRow>(&json)
        {
            if p.session_id == body.session_id {
                if p.user_id != user.id {
                    return Err(AppError::Forbidden(
                        "Session belongs to another user".into(),
                    ));
                }
                found_sender = true;
            }
            if p.session_id == body.target_session_id {
                found_target = true;
            }
        }
    }

    if !found_sender {
        return Err(AppError::Forbidden(
            "Sender is not an active participant in this call".into(),
        ));
    }
    if !found_target {
        return Err(AppError::NotFound(
            "Target participant not in call".into(),
        ));
    }

    // ローカル宛先のバッファ満杯チェック ([P2] 503 返却)
    if state
        .call_registry
        .is_target_full(&body.target_session_id)
        .await
    {
        return Err(AppError::ServiceUnavailable(
            "Signaling buffer full for target participant".into(),
        ));
    }

    // 送信元の有効期限を延長
    let _ = toasty::sql::statement(
        "UPDATE call_presence
         SET expires_at = statement_timestamp() + interval '30 seconds'
         WHERE channel_id = $1 AND session_id = $2::uuid",
    )
    .bind(channel_id.clone())
    .bind(body.session_id.to_string())
    .exec(&mut db)
    .await;

    // 分散イベント通知を発行
    let event = CallDistributedEvent::Signal {
        channel_id,
        target_session_id: body.target_session_id,
        from_session_id: body.session_id,
        signal: body.signal,
    };
    CallRegistry::notify_event(&mut db, &event).await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn update_state(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<UpdateStatePayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;

    let rows = toasty::sql::query(
        "UPDATE call_presence
         SET is_muted = $4, expires_at = statement_timestamp() + interval '30 seconds'
         WHERE channel_id = $1 AND session_id = $2::uuid AND user_id = $3 AND expires_at > statement_timestamp()
         RETURNING json_build_object(
             'session_id', session_id,
             'user_id', user_id,
             'user_name', user_name,
             'display_name', display_name,
             'is_muted', is_muted,
             'joined_at', joined_at
         )::text",
    )
    .bind(channel_id.clone())
    .bind(body.session_id.to_string())
    .bind(user.id.clone())
    .bind(body.is_muted)
    .exec(&mut db)
    .await?;

    let updated_participant = rows.into_iter().find_map(|row| {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
        {
            serde_json::from_str::<CallParticipant>(&json).ok()
        } else {
            None
        }
    });

    let Some(participant) = updated_participant else {
        return Err(AppError::NotFound("Session not found in call".into()));
    };

    let event = CallDistributedEvent::ParticipantUpdated {
        channel_id,
        participant,
    };
    CallRegistry::notify_event(&mut db, &event).await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn heartbeat(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<HeartbeatPayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;

    let rows = toasty::sql::query(
        "UPDATE call_presence
         SET expires_at = statement_timestamp() + interval '30 seconds'
         WHERE channel_id = $1 AND session_id = $2::uuid AND user_id = $3 AND expires_at > statement_timestamp()
         RETURNING session_id::text",
    )
    .bind(channel_id)
    .bind(body.session_id.to_string())
    .bind(user.id)
    .exec(&mut db)
    .await?;

    if rows.is_empty() {
        return Err(AppError::NotFound("Session not found in call".into()));
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn leave_call(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<LeavePayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut db = state.db.clone();
    authorize_call_access(&mut db, &channel_id, &user).await?;

    let rows = toasty::sql::query(
        "DELETE FROM call_presence
         WHERE channel_id = $1 AND session_id = $2::uuid AND user_id = $3
         RETURNING user_id",
    )
    .bind(channel_id.clone())
    .bind(body.session_id.to_string())
    .bind(user.id.clone())
    .exec(&mut db)
    .await?;

    if rows.is_empty() {
        return Err(AppError::NotFound("Session not found in call".into()));
    }

    state
        .call_registry
        .unregister_local_session(&body.session_id)
        .await;

    let event = CallDistributedEvent::ParticipantLeft {
        channel_id,
        session_id: body.session_id,
        user_id: user.id,
    };
    CallRegistry::notify_event(&mut db, &event).await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_active_calls(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<ActiveCallSummary>>, AppError> {
    let mut db = state.db.clone();

    // 期限切れレコードのクリーンアップスイープ
    let _ = CallRegistry::sweep_stale_presence(&mut db).await;

    #[derive(Deserialize)]
    struct ActiveRow {
        channel_id: String,
        user_id: String,
        user_name: String,
        display_name: String,
        is_muted: bool,
        joined_at: String,
    }

    let rows = toasty::sql::query(
        "SELECT json_build_object(
            'channel_id', channel_id,
            'user_id', user_id,
            'user_name', user_name,
            'display_name', display_name,
            'is_muted', is_muted,
            'joined_at', joined_at
         )::text
         FROM call_presence
         WHERE expires_at > statement_timestamp()
         ORDER BY channel_id, joined_at ASC",
    )
    .exec(&mut db)
    .await?;

    let mut channel_map: HashMap<String, Vec<PublicCallParticipant>> = HashMap::new();
    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
            && let Ok(r) = serde_json::from_str::<ActiveRow>(&json)
        {
            channel_map.entry(r.channel_id).or_default().push(PublicCallParticipant {
                user_id: r.user_id,
                user_name: r.user_name,
                display_name: r.display_name,
                is_muted: r.is_muted,
                joined_at: r.joined_at,
            });
        }
    }

    let mut active = Vec::new();
    for (cid, participants) in channel_map {
        if authorize_call_access(&mut db, &cid, &user).await.is_ok() && !participants.is_empty() {
            active.push(ActiveCallSummary {
                channel_id: cid,
                participant_count: participants.len(),
                participants,
            });
        }
    }

    active.sort_by(|a, b| a.channel_id.cmp(&b.channel_id));
    Ok(Json(active))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_call_registry_dispatch_events() {
        let registry = CallRegistry::new();
        let (tx1, mut rx1) = mpsc::channel(256);
        let session_1 = Uuid::new_v4();

        registry
            .register_local_session(session_1, "chan-1".into(), "user-1".into(), tx1)
            .await;

        let session_2 = Uuid::new_v4();
        let p2 = CallParticipant {
            session_id: session_2,
            user_id: "user-2".into(),
            user_name: "bob".into(),
            display_name: "Bob".into(),
            is_muted: false,
            joined_at: "2026-10-28T00:00:01Z".into(),
        };

        // ParticipantJoined イベントのディスパッチテスト
        let join_event = CallDistributedEvent::ParticipantJoined {
            channel_id: "chan-1".into(),
            participant: p2.clone(),
        };
        let join_json = serde_json::to_string(&join_event).unwrap();
        registry.dispatch_distributed_event(&join_json).await;

        let ev1 = rx1.recv().await.expect("should receive event");
        match ev1 {
            CallServerEvent::ParticipantJoined { participant } => {
                assert_eq!(participant.session_id, session_2);
            }
            _ => panic!("expected ParticipantJoined event"),
        }

        // Signal イベントのディスパッチテスト
        let sig_event = CallDistributedEvent::Signal {
            channel_id: "chan-1".into(),
            target_session_id: session_1,
            from_session_id: session_2,
            signal: serde_json::json!({"type": "offer", "sdp": "v=0..."}),
        };
        let sig_json = serde_json::to_string(&sig_event).unwrap();
        registry.dispatch_distributed_event(&sig_json).await;

        let ev2 = rx1.recv().await.expect("should receive event");
        match ev2 {
            CallServerEvent::Signal {
                from_session_id,
                signal,
            } => {
                assert_eq!(from_session_id, session_2);
                assert_eq!(signal["type"], "offer");
            }
            _ => panic!("expected Signal event"),
        }

        // ParticipantLeft イベントのディスパッチテスト
        let left_event = CallDistributedEvent::ParticipantLeft {
            channel_id: "chan-1".into(),
            session_id: session_2,
            user_id: "user-2".into(),
        };
        let left_json = serde_json::to_string(&left_event).unwrap();
        registry.dispatch_distributed_event(&left_json).await;

        let ev3 = rx1.recv().await.expect("should receive event");
        match ev3 {
            CallServerEvent::ParticipantLeft { session_id, .. } => {
                assert_eq!(session_id, session_2);
            }
            _ => panic!("expected ParticipantLeft event"),
        }
    }
}
