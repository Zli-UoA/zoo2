//! Authenticated WebRTC control for private server-side SIP registrations.
use crate::{
    apps::{one, query},
    auth::AuthUser,
    error::AppError,
    phone_gateway::Gateway,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, time::Duration};
use tokio::sync::mpsc;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/phone/sessions/{id}/stream", post(events))
        .route("/api/phone/sessions/{id}/command", post(command))
}
fn missing() -> AppError {
    AppError::NotFound("Phone session unavailable".into())
}
fn gateway(state: &AppState) -> Result<Gateway, AppError> {
    Gateway::new(state.phone_http_client.clone(), &state.config.phone_gateway)
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}
fn sid(row: &Value) -> Result<u64, AppError> {
    row["gateway_session_id"].as_u64().ok_or_else(missing)
}
fn hid(row: &Value) -> Result<u64, AppError> {
    row["gateway_handle_id"].as_u64().ok_or_else(missing)
}

// Every action joins from one session PK to one user/line PK and one membership
// PK. Revocation and edited credentials are checked on the server, including ICE.
async fn owned(state: &AppState, user: &str, id: &str) -> Result<Value, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(&mut tx, "SELECT json_build_object('id',s.id,'peer_id',s.peer_id,'gateway_session_id',s.gateway_session_id,'gateway_handle_id',s.gateway_handle_id,'sip_domain',l.sip_domain)::text FROM phone_sessions s JOIN sip_lines l ON l.id=s.line_id JOIN users u ON u.id=s.user_id WHERE s.id=$1 AND s.user_id=$2 AND s.expires_at>now() AND s.line_revision=l.updated_at AND NOT u.is_bot AND NOT u.is_deactivated AND (l.owner_user_id=s.user_id OR (l.owner_user_id IS NULL AND EXISTS(SELECT 1 FROM sip_line_members m WHERE m.line_id=l.id AND m.user_id=s.user_id)))", &[id,user]).await.map_err(|error| match error { AppError::NotFound(_) => missing(), other => other })
}

/// The reservation precedes network I/O and is capped at four live sessions per
/// user, across all replicas. Expired reservations never accumulate per user.
pub async fn connect(
    state: &AppState,
    user: &AuthUser,
    line: &str,
    settings: &Value,
    password: &str,
    ice: Value,
) -> Result<Value, AppError> {
    let gateway = gateway(state)?;
    let id = uuid::Uuid::new_v4().to_string();
    let peer = uuid::Uuid::new_v4().to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(&mut tx, "SELECT to_json(id)::text FROM users WHERE id=$1 AND NOT is_bot AND NOT is_deactivated FOR NO KEY UPDATE", &[&user.id]).await?;
    // Parent line precedes session rows, matching line deletion/cascades.
    let row = one(&mut tx, "SELECT json_build_object('revision',updated_at::text)::text FROM sip_lines l WHERE id=$1 AND updated_at=$3::timestamptz AND (owner_user_id=$2 OR (owner_user_id IS NULL AND EXISTS(SELECT 1 FROM sip_line_members m WHERE m.line_id=l.id AND m.user_id=$2))) FOR SHARE OF l", &[line,&user.id,text(settings,"revision")]).await?;
    let expired = query(&mut tx, "DELETE FROM phone_sessions WHERE user_id=$1 AND expires_at<=now() RETURNING json_build_object('gateway_session_id',gateway_session_id)::text", &[&user.id]).await?;
    let count = one(
        &mut tx,
        "SELECT to_json(count(*))::text FROM phone_sessions WHERE user_id=$1 AND expires_at>now()",
        &[&user.id],
    )
    .await?;
    if count.as_u64().unwrap_or(4) >= 4 {
        return Err(AppError::Conflict("Too many phone sessions".into()));
    }

    toasty::sql::statement("INSERT INTO phone_sessions(id,user_id,line_id,line_revision,peer_id) VALUES($1,$2,$3,$4::timestamptz,$5)")
        .bind(id.clone()).bind(user.id.clone()).bind(line.to_owned()).bind(text(&row,"revision").to_owned()).bind(peer.clone()).exec(&mut tx).await?;
    tx.commit().await?;
    for old in expired {
        if let Ok(old_id) = sid(&old) {
            let g = gateway.clone();
            tokio::spawn(async move {
                g.destroy(old_id).await;
            });
        }
    }
    let mut created = None;
    let result = async {
        let session = gateway.create().await?; created = Some(session);
        let handle = gateway.attach(session).await?;
        let aor = if state.config.phone_gateway.pbx_config_secret.is_some() { format!("line-{}", text(settings,"id")) } else { text(settings,"username").to_owned() };
        gateway.register(session,handle,text(settings,"sip_domain"),text(settings,"username"),&aor,password).await?;
        let mut tx = db.transaction().await?;
        one(&mut tx, "UPDATE phone_sessions SET gateway_session_id=$3::bigint,gateway_handle_id=$4::bigint WHERE id=$1 AND user_id=$2 AND expires_at>now() RETURNING to_json(id)::text", &[&id,&user.id,&session.to_string(),&handle.to_string()]).await?;
        tx.commit().await?;
        Ok::<_,AppError>(json!({"session_id":id,"peer_id":peer,"ice_servers":ice,"relay_only":state.config.phone_gateway.relay_only(),"revision":row["revision"]}))
    }.await;
    if result.is_err() {
        let _ = toasty::sql::statement("DELETE FROM phone_sessions WHERE id=$1 AND user_id=$2")
            .bind(id)
            .bind(user.id.clone())
            .exec(&mut db)
            .await;
        if let Some(session) = created {
            gateway.destroy(session).await;
        }
    }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    action: String,
    number: Option<String>,
    signal: Option<Value>,
    tone: Option<String>,
}
fn sdp(signal: Option<&Value>, kind: &str) -> Result<Value, AppError> {
    let value = signal.ok_or_else(|| AppError::BadRequest("WebRTC description required".into()))?;
    let body = text(value, "sdp");
    if text(value, "type") != kind || body.is_empty() || body.len() > 65_536 {
        return Err(AppError::BadRequest("Invalid WebRTC description".into()));
    }
    Ok(json!({"type":kind,"sdp":body}))
}
fn telephone(value: &str) -> bool {
    let number = value.strip_prefix('+').unwrap_or(value);
    !number.is_empty() && number.len() <= 32 && number.bytes().all(|b| b.is_ascii_digit())
}
async fn command(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<Command>,
) -> Result<impl IntoResponse, AppError> {
    let row = owned(&state, &user.id, &id).await?;
    let gateway = gateway(&state)?;
    let (session, handle) = (sid(&row)?, hid(&row)?);
    match body.action.as_str() {
        "call" => {
            let number = body
                .number
                .as_deref()
                .filter(|n| telephone(n))
                .ok_or_else(|| AppError::BadRequest("Invalid telephone number".into()))?;
            let offer = sdp(body.signal.as_ref(), "offer")?;
            gateway.message(session,handle,json!({"request":"call","uri":format!("sip:{number}@{}",text(&row,"sip_domain"))}),Some(offer)).await?;
        }
        "accept" => {
            gateway
                .message(
                    session,
                    handle,
                    json!({"request":"accept"}),
                    Some(sdp(body.signal.as_ref(), "answer")?),
                )
                .await?
        }
        "candidate" => {
            let signal = body
                .signal
                .ok_or_else(|| AppError::BadRequest("WebRTC candidate required".into()))?;
            let candidate = &signal["candidate"];
            if !candidate.is_object()
                || candidate.to_string().len() > 8192
                || !candidate["candidate"].is_string()
            {
                return Err(AppError::BadRequest("Invalid WebRTC candidate".into()));
            }
            gateway.trickle(session, handle, candidate.clone()).await?;
        }
        "dtmf" => {
            let tone = body
                .tone
                .filter(|s| s.len() == 1 && s.bytes().all(|c| b"0123456789*#".contains(&c)))
                .ok_or_else(|| AppError::BadRequest("Invalid telephone tone".into()))?;
            gateway
                .message(
                    session,
                    handle,
                    json!({"request":"dtmf_info","digit":tone}),
                    None,
                )
                .await?;
        }
        "hangup" => {
            gateway
                .message(session, handle, json!({"request":"hangup"}), None)
                .await?
        }
        "decline" => {
            gateway
                .message(
                    session,
                    handle,
                    json!({"request":"decline","code":486}),
                    None,
                )
                .await?
        }
        "disconnect" => {
            let mut db = state.db.clone();
            toasty::sql::statement("DELETE FROM phone_sessions WHERE id=$1 AND user_id=$2")
                .bind(id)
                .bind(user.id)
                .exec(&mut db)
                .await?;
            gateway.destroy(session).await;
        }
        _ => return Err(AppError::BadRequest("Invalid phone operation".into())),
    }
    Ok(super::sip::private(json!({"data":{},"txid":null})))
}

/// Only explicitly selected fields cross back to clients; plugin errors,
/// registrar credentials, SIP headers and internal Janus IDs never do.
fn client_events(
    value: &Value,
    handle: u64,
    peer: &str,
) -> Result<Vec<(&'static str, Value)>, AppError> {
    if value["sender"].as_u64().is_some_and(|id| id != handle) {
        return Err(missing());
    }
    let mut events = Vec::new();
    match value["janus"].as_str() {
        Some("detached" | "timeout" | "error") => return Ok(vec![("disconnected", json!({}))]),
        Some("trickle") => {
            if value["candidate"].is_object() && value["candidate"]["candidate"].is_string() {
                events.push(("signal",json!({"from_session_id":peer,"signal":{"type":"candidate","candidate":value["candidate"]}})));
            }
        }
        // SIP plugin termination is authoritative. Janus also emits a core
        // hangup for the same PeerConnection; forwarding both can end a new call.
        Some("hangup") => {}
        Some("event") => {
            let data = &value["plugindata"]["data"];
            if !data["error_code"].is_null() {
                return Ok(vec![("disconnected", json!({}))]);
            }
            let kind = text(&data["result"], "event");
            if kind == "incomingcall" {
                events.push((
                    "incoming",
                    json!({"signal":sdp(Some(&value["jsep"]),"offer")?}),
                ));
            } else {
                if !value["jsep"].is_null() {
                    events.push(("signal",json!({"from_session_id":peer,"signal":sdp(Some(&value["jsep"]),"answer")?})));
                }
                match kind {
                    "registered" => events.push(("registered", json!({}))),
                    "accepted" => events.push(("accepted", json!({}))),
                    "hangup" => events.push(("ended", json!({}))),
                    "unregistered" | "registration_failed" => {
                        events.push(("disconnected", json!({})))
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Ok(events)
}

struct StreamGuard {
    state: AppState,
    id: String,
    owner: String,
    gateway: Gateway,
    session: u64,
}
impl Drop for StreamGuard {
    fn drop(&mut self) {
        let (state, id, owner, gateway, session) = (
            self.state.clone(),
            self.id.clone(),
            self.owner.clone(),
            self.gateway.clone(),
            self.session,
        );
        tokio::spawn(async move {
            let mut db = state.db.clone();
            let Ok(mut tx) = db.transaction().await else {
                return;
            };
            let Ok(rows)=query(&mut tx,"SELECT json_build_object('lease_owner',lease_owner)::text FROM phone_sessions WHERE id=$1 FOR UPDATE",&[&id]).await else { return };
            if rows.first().is_some_and(|r| r["lease_owner"] != owner) {
                return;
            }
            if toasty::sql::statement("DELETE FROM phone_sessions WHERE id=$1")
                .bind(id)
                .exec(&mut tx)
                .await
                .is_err()
            {
                return;
            }
            if tx.commit().await.is_ok() {
                gateway.destroy(session).await;
            }
        });
    }
}
async fn events(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let row = owned(&state, &user.id, &id).await?;
    let gateway = gateway(&state)?;
    let (session, handle) = (sid(&row)?, hid(&row)?);
    let peer = text(&row, "peer_id").to_owned();
    let owner = uuid::Uuid::new_v4().to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(&mut tx,"UPDATE phone_sessions SET lease_owner=$3,expires_at=now()+interval '45 seconds' WHERE id=$1 AND user_id=$2 AND (lease_owner IS NULL OR expires_at<=now()) RETURNING to_json(id)::text",&[&id,&user.id,&owner]).await.map_err(|_| AppError::Conflict("Phone stream already connected".into()))?;
    tx.commit().await?;
    let (send, mut receive) = mpsc::channel::<(&'static str, Value)>(64);
    let guard = StreamGuard {
        state: state.clone(),
        id: id.clone(),
        owner: owner.clone(),
        gateway: gateway.clone(),
        session,
    };
    tokio::spawn(async move {
        let _guard = guard;
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        let mut poll = Box::pin(gateway.poll(session));
        let result=async {
            loop {
                tokio::select! {
                    _=send.closed()=>return Ok::<_,AppError>(()),
                    _=interval.tick()=> {
                        owned(&state,&user.id,&id).await?;
                        let mut db=state.db.clone(); let mut tx=db.transaction().await?;
                        one(&mut tx,"UPDATE phone_sessions SET expires_at=now()+interval '45 seconds' WHERE id=$1 AND user_id=$2 AND lease_owner=$3 AND expires_at>now() RETURNING to_json(id)::text",&[&id,&user.id,&owner]).await?;
                        tx.commit().await?;
                        gateway.keepalive(session).await?;
                    }
                    response=&mut poll=> {
                        for (event,data) in client_events(&response?,handle,&peer)? {
                            let terminal=event=="disconnected";
                            // Slow or abandoned clients release the registration.
                            send.try_send((event,data)).map_err(|_| missing())?;
                            if terminal { return Ok(()) }
                        }
                        poll=Box::pin(gateway.poll(session));
                    }
                }
            }
        }.await;
        if result.is_err() {
            let _ = send.try_send(("disconnected", json!({})));
        }
    });
    let stream =
        futures_util::stream::poll_fn(move |cx| receive.poll_recv(cx)).map(|(event, data)| {
            Ok::<_, Infallible>(Event::default().event(event).data(data.to_string()))
        });
    use futures_util::StreamExt;
    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_messages_are_projected_to_webrtc_without_sip_or_credentials() {
        let event = json!({"janus":"event","sender":1,"jsep":{"type":"offer","sdp":"v=0\r\n"},"plugindata":{"data":{"result":{"event":"incomingcall","username":"sip:private@pbx","secret":"dummy-private"}}}});
        let messages = client_events(&event, 1, "peer").unwrap();
        assert_eq!(messages[0].0, "incoming");
        let encoded = serde_json::to_string(&messages).unwrap();
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains("sip:"));
        assert!(client_events(&event, 2, "peer").is_err());
        assert!(
            client_events(&json!({"janus":"hangup","sender":1}), 1, "peer")
                .unwrap()
                .is_empty()
        );
        let terminated = json!({"janus":"event","sender":1,"plugindata":{"data":{"result":{"event":"hangup"},"call_id":"private-sip-call"}}});
        assert_eq!(
            client_events(&terminated, 1, "peer").unwrap(),
            vec![("ended", json!({}))]
        );
    }
    #[test]
    fn descriptions_and_dial_targets_are_bounded() {
        assert!(telephone("+81312345678"));
        assert!(!telephone("+"));
        assert!(!telephone("123@evil"));
        assert!(sdp(Some(&json!({"type":"offer","sdp":"v=0"})), "offer").is_ok());
        assert!(sdp(Some(&json!({"type":"answer","sdp":"v=0"})), "offer").is_err());
    }
}
