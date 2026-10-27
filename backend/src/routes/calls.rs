use std::{
    collections::HashMap,
    convert::Infallible,
    sync::Arc,
    time::{Duration, Instant},
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
pub const MAX_SIGNAL_PAYLOAD_SIZE: usize = 65536;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallParticipant {
    pub session_id: Uuid,
    pub user_id: String,
    pub user_name: String,
    pub display_name: String,
    pub is_muted: bool,
    pub joined_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallStatusResponse {
    pub is_active: bool,
    pub channel_id: String,
    pub participant_count: usize,
    pub participants: Vec<CallParticipant>,
    pub max_participants: usize,
    pub ice_servers: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveCallSummary {
    pub channel_id: String,
    pub participant_count: usize,
    pub participants: Vec<CallParticipant>,
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

struct ParticipantSession {
    participant: CallParticipant,
    sender: mpsc::UnboundedSender<CallServerEvent>,
    #[allow(dead_code)]
    last_heartbeat: Instant,
}

struct CallRoom {
    participants: HashMap<Uuid, ParticipantSession>,
}

#[derive(Clone, Default)]
pub struct CallRegistry {
    rooms: Arc<RwLock<HashMap<String, CallRoom>>>,
}

impl CallRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get_participants(&self, channel_id: &str) -> Vec<CallParticipant> {
        let rooms = self.rooms.read().await;
        rooms
            .get(channel_id)
            .map(|r| r.participants.values().map(|p| p.participant.clone()).collect())
            .unwrap_or_default()
    }

    pub async fn get_active_channel_ids(&self) -> Vec<String> {
        let rooms = self.rooms.read().await;
        rooms
            .iter()
            .filter(|(_, r)| !r.participants.is_empty())
            .map(|(cid, _)| cid.clone())
            .collect()
    }

    pub async fn join(
        &self,
        channel_id: &str,
        participant: CallParticipant,
        sender: mpsc::UnboundedSender<CallServerEvent>,
        max_participants: usize,
        ice_servers: serde_json::Value,
    ) -> Result<(), AppError> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .entry(channel_id.to_string())
            .or_insert_with(|| CallRoom {
                participants: HashMap::new(),
            });

        let session_id = participant.session_id;

        // すでに存在しない新規セッションの場合に上限をチェック
        if !room.participants.contains_key(&session_id) && room.participants.len() >= max_participants {
            return Err(AppError::Conflict(format!(
                "Call room is full (max {max_participants} participants)"
            )));
        }

        // 既存の参加者一覧を収集
        let existing_participants: Vec<CallParticipant> = room
            .participants
            .values()
            .map(|p| p.participant.clone())
            .collect();

        // 他の既存参加者へ participant_joined を配信
        let join_event = CallServerEvent::ParticipantJoined {
            participant: participant.clone(),
        };
        for (sid, p) in &room.participants {
            if *sid != session_id {
                let _ = p.sender.send(join_event.clone());
            }
        }

        // 新規参加者自身の room_state イベント用の参加者一覧（自分を含む）
        let mut all_participants = existing_participants;
        if !all_participants.iter().any(|p| p.session_id == session_id) {
            all_participants.push(participant.clone());
        }

        let room_state_event = CallServerEvent::RoomState {
            participants: all_participants,
            max_participants,
            ice_servers,
        };
        let _ = sender.send(room_state_event);

        room.participants.insert(
            session_id,
            ParticipantSession {
                participant,
                sender,
                last_heartbeat: Instant::now(),
            },
        );

        Ok(())
    }

    pub async fn send_signal(
        &self,
        channel_id: &str,
        sender_session_id: Uuid,
        target_session_id: Uuid,
        signal: serde_json::Value,
    ) -> Result<(), AppError> {
        let rooms = self.rooms.read().await;
        let room = rooms
            .get(channel_id)
            .ok_or_else(|| AppError::NotFound("Call room not active".into()))?;

        if !room.participants.contains_key(&sender_session_id) {
            return Err(AppError::Forbidden(
                "Sender is not a participant in this call".into(),
            ));
        }

        if let Some(target) = room.participants.get(&target_session_id) {
            let event = CallServerEvent::Signal {
                from_session_id: sender_session_id,
                signal,
            };
            let _ = target.sender.send(event);
        }

        Ok(())
    }

    pub async fn update_state(
        &self,
        channel_id: &str,
        session_id: Uuid,
        user_id: &str,
        is_muted: bool,
    ) -> Result<(), AppError> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(channel_id)
            .ok_or_else(|| AppError::NotFound("Call room not active".into()))?;

        let session = room
            .participants
            .get_mut(&session_id)
            .ok_or_else(|| AppError::NotFound("Session not found in call".into()))?;

        if session.participant.user_id != user_id {
            return Err(AppError::Forbidden("Session belongs to another user".into()));
        }

        session.participant.is_muted = is_muted;
        session.last_heartbeat = Instant::now();

        let updated_event = CallServerEvent::ParticipantUpdated {
            participant: session.participant.clone(),
        };

        for (sid, p) in &room.participants {
            if *sid != session_id {
                let _ = p.sender.send(updated_event.clone());
            }
        }

        Ok(())
    }

    pub async fn leave(&self, channel_id: &str, session_id: Uuid) -> Option<CallParticipant> {
        let mut rooms = self.rooms.write().await;
        let room = rooms.get_mut(channel_id)?;

        let removed = room.participants.remove(&session_id);
        if let Some(ref session) = removed {
            let left_event = CallServerEvent::ParticipantLeft {
                session_id,
                user_id: session.participant.user_id.clone(),
            };
            for p in room.participants.values() {
                let _ = p.sender.send(left_event.clone());
            }
        }

        if room.participants.is_empty() {
            rooms.remove(channel_id);
        }

        removed.map(|s| s.participant)
    }

    pub async fn close_room(&self, channel_id: &str) {
        let mut rooms = self.rooms.write().await;
        rooms.remove(channel_id);
    }

    pub async fn remove_user_from_channel(&self, channel_id: &str, user_id: &str) {
        let mut rooms = self.rooms.write().await;
        let Some(room) = rooms.get_mut(channel_id) else {
            return;
        };

        let session_ids_to_remove: Vec<Uuid> = room
            .participants
            .iter()
            .filter(|(_, p)| p.participant.user_id == user_id)
            .map(|(sid, _)| *sid)
            .collect();

        for sid in session_ids_to_remove {
            if let Some(session) = room.participants.remove(&sid) {
                let left_event = CallServerEvent::ParticipantLeft {
                    session_id: sid,
                    user_id: session.participant.user_id,
                };
                for p in room.participants.values() {
                    let _ = p.sender.send(left_event.clone());
                }
            }
        }

        if room.participants.is_empty() {
            rooms.remove(channel_id);
        }
    }
}

pub struct SessionGuard {
    call_registry: CallRegistry,
    channel_id: String,
    session_id: Uuid,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let registry = self.call_registry.clone();
        let channel_id = self.channel_id.clone();
        let session_id = self.session_id;
        tokio::spawn(async move {
            registry.leave(&channel_id, session_id).await;
        });
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{id}/call", get(get_call_status))
        .route("/api/channels/{id}/call/stream", get(call_stream))
        .route("/api/channels/{id}/call/signal", post(send_signal))
        .route("/api/channels/{id}/call/state", post(update_state))
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

    let ice_servers: serde_json::Value = serde_json::from_str(&state.config.webrtc_ice_servers_json)
        .unwrap_or_else(|_| serde_json::json!([{"urls": ["stun:stun.l.google.com:19302"]}]));

    let participants = state.call_registry.get_participants(&channel_id).await;
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

    let now_str = crate::models::now_utc_millis().to_string();
    let participant = CallParticipant {
        session_id: query.session_id,
        user_id: user.id.clone(),
        user_name: user.user_name.clone(),
        display_name: user.display_name.clone(),
        is_muted: false,
        joined_at: now_str,
    };

    let (tx, rx) = mpsc::unbounded_channel();
    let ice_servers: serde_json::Value = serde_json::from_str(&state.config.webrtc_ice_servers_json)
        .unwrap_or_else(|_| serde_json::json!([{"urls": ["stun:stun.l.google.com:19302"]}]));

    state
        .call_registry
        .join(
            &channel_id,
            participant,
            tx,
            MAX_CALL_PARTICIPANTS,
            ice_servers,
        )
        .await?;

    let guard = SessionGuard {
        call_registry: state.call_registry.clone(),
        channel_id: channel_id.clone(),
        session_id: query.session_id,
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

    let payload_len = serde_json::to_string(&body.signal).unwrap_or_default().len();
    if payload_len > MAX_SIGNAL_PAYLOAD_SIZE {
        return Err(AppError::BadRequest("Signal payload too large".into()));
    }

    state
        .call_registry
        .send_signal(
            &channel_id,
            body.session_id,
            body.target_session_id,
            body.signal,
        )
        .await?;

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

    state
        .call_registry
        .update_state(&channel_id, body.session_id, &user.id, body.is_muted)
        .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn leave_call(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(channel_id): Path<String>,
    Json(body): Json<LeavePayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.call_registry.leave(&channel_id, body.session_id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_active_calls(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<ActiveCallSummary>>, AppError> {
    let channel_ids = state.call_registry.get_active_channel_ids().await;
    let mut db = state.db.clone();
    let mut active = Vec::new();

    for cid in channel_ids {
        if authorize_call_access(&mut db, &cid, &user).await.is_ok() {
            let participants = state.call_registry.get_participants(&cid).await;
            if !participants.is_empty() {
                active.push(ActiveCallSummary {
                    channel_id: cid,
                    participant_count: participants.len(),
                    participants,
                });
            }
        }
    }

    Ok(Json(active))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_call_registry_join_and_room_state() {
        let registry = CallRegistry::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let session_1 = Uuid::new_v4();

        let p1 = CallParticipant {
            session_id: session_1,
            user_id: "user-1".into(),
            user_name: "alice".into(),
            display_name: "Alice".into(),
            is_muted: false,
            joined_at: "2026-10-28T00:00:00Z".into(),
        };

        let ice = serde_json::json!([{"urls": ["stun:stun.l.google.com:19302"]}]);
        registry
            .join("chan-1", p1.clone(), tx1, 6, ice.clone())
            .await
            .expect("join should succeed");

        // rx1 should receive RoomState with p1
        let first_event = rx1.recv().await.expect("should receive event");
        match first_event {
            CallServerEvent::RoomState { participants, .. } => {
                assert_eq!(participants.len(), 1);
                assert_eq!(participants[0].session_id, session_1);
            }
            _ => panic!("expected RoomState event"),
        }

        // Second user joins
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let session_2 = Uuid::new_v4();
        let p2 = CallParticipant {
            session_id: session_2,
            user_id: "user-2".into(),
            user_name: "bob".into(),
            display_name: "Bob".into(),
            is_muted: false,
            joined_at: "2026-10-28T00:00:01Z".into(),
        };

        registry
            .join("chan-1", p2.clone(), tx2, 6, ice)
            .await
            .expect("join should succeed");

        // rx1 should receive ParticipantJoined for p2
        let join_event = rx1.recv().await.expect("should receive event");
        match join_event {
            CallServerEvent::ParticipantJoined { participant } => {
                assert_eq!(participant.session_id, session_2);
            }
            _ => panic!("expected ParticipantJoined event"),
        }

        // rx2 should receive RoomState with both p1 and p2
        let state2 = rx2.recv().await.expect("should receive event");
        match state2 {
            CallServerEvent::RoomState { participants, .. } => {
                assert_eq!(participants.len(), 2);
            }
            _ => panic!("expected RoomState event"),
        }
    }

    #[tokio::test]
    async fn test_call_registry_max_capacity() {
        let registry = CallRegistry::new();
        let ice = serde_json::json!([{"urls": ["stun:stun.l.google.com:19302"]}]);

        for i in 0..6 {
            let (tx, _rx) = mpsc::unbounded_channel();
            let p = CallParticipant {
                session_id: Uuid::new_v4(),
                user_id: format!("user-{i}"),
                user_name: format!("user{i}"),
                display_name: format!("User {i}"),
                is_muted: false,
                joined_at: "2026-10-28T00:00:00Z".into(),
            };
            registry
                .join("chan-cap", p, tx, 6, ice.clone())
                .await
                .expect("should succeed within capacity");
        }

        // 7th should fail with Conflict
        let (tx7, _rx7) = mpsc::unbounded_channel();
        let p7 = CallParticipant {
            session_id: Uuid::new_v4(),
            user_id: "user-7".into(),
            user_name: "user7".into(),
            display_name: "User 7".into(),
            is_muted: false,
            joined_at: "2026-10-28T00:00:00Z".into(),
        };
        let err = registry
            .join("chan-cap", p7, tx7, 6, ice)
            .await
            .expect_err("should reject 7th participant");

        match err {
            AppError::Conflict(msg) => assert!(msg.contains("full")),
            _ => panic!("expected Conflict error"),
        }
    }

    #[tokio::test]
    async fn test_call_registry_signaling_and_state_updates() {
        let registry = CallRegistry::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        let ice = serde_json::json!([]);

        let p1 = CallParticipant {
            session_id: s1,
            user_id: "u1".into(),
            user_name: "u1".into(),
            display_name: "U1".into(),
            is_muted: false,
            joined_at: "2026-10-28T00:00:00Z".into(),
        };
        let p2 = CallParticipant {
            session_id: s2,
            user_id: "u2".into(),
            user_name: "u2".into(),
            display_name: "U2".into(),
            is_muted: false,
            joined_at: "2026-10-28T00:00:00Z".into(),
        };

        registry.join("chan-sig", p1, tx1, 6, ice.clone()).await.unwrap();
        let _ = rx1.recv().await; // room_state
        registry.join("chan-sig", p2, tx2, 6, ice).await.unwrap();
        let _ = rx1.recv().await; // p2 joined
        let _ = rx2.recv().await; // room_state

        // s1 sends signal to s2
        let sig = serde_json::json!({"type": "offer", "sdp": "v=0..."});
        registry
            .send_signal("chan-sig", s1, s2, sig.clone())
            .await
            .unwrap();

        let received = rx2.recv().await.unwrap();
        match received {
            CallServerEvent::Signal {
                from_session_id,
                signal,
            } => {
                assert_eq!(from_session_id, s1);
                assert_eq!(signal, sig);
            }
            _ => panic!("expected Signal event"),
        }

        // s1 mutes
        registry
            .update_state("chan-sig", s1, "u1", true)
            .await
            .unwrap();

        let updated = rx2.recv().await.unwrap();
        match updated {
            CallServerEvent::ParticipantUpdated { participant } => {
                assert_eq!(participant.session_id, s1);
                assert!(participant.is_muted);
            }
            _ => panic!("expected ParticipantUpdated event"),
        }

        // s2 leaves
        registry.leave("chan-sig", s2).await;
        let left = rx1.recv().await.unwrap();
        match left {
            CallServerEvent::ParticipantLeft { session_id, .. } => {
                assert_eq!(session_id, s2);
            }
            _ => panic!("expected ParticipantLeft event"),
        }

        assert_eq!(registry.get_participants("chan-sig").await.len(), 1);
    }
}
