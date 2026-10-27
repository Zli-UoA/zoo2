use super::*;
use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, KeyInit, Mac};
use std::{net::IpAddr, time::Duration};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/v1/socket-ticket", post(ticket))
        .route("/api/apps/v1/socket", get(socket))
        .layer(axum::middleware::map_response(super::no_store))
}
#[derive(Deserialize)]
struct TicketQuery {
    ticket: String,
}
async fn ticket(State(s): State<AppState>, p: Principal) -> Result<Json<Value>, AppError> {
    if p.delegated {
        return Err(AppError::Forbidden(
            "Use a bot token for event delivery".into(),
        ));
    }
    if p.manifest.transport != Transport::Websocket {
        return Err(AppError::BadRequest("App uses HTTP delivery".into()));
    }
    let raw = secret("zoo_ws_");
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("INSERT INTO app_socket_tickets(ticket_hash,credential_id,expires_at) VALUES($1,$2,now()+interval '30 seconds')").bind(hash(&raw)).bind(p.credential_id).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"ticket":raw,"expires_in":30,"path":"/api/apps/v1/socket"}),
    ))
}
async fn socket(
    State(s): State<AppState>,
    Query(q): Query<TicketQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let v=one(&mut tx,"UPDATE app_socket_tickets SET used_at=now() WHERE ticket_hash=$1 AND expires_at>now() AND used_at IS NULL RETURNING json_build_object('credential',credential_id)::text",&[&hash(&q.ticket)]).await?;
    let credential = v["credential"].as_str().unwrap().to_owned();
    let v=one(&mut tx,"SELECT json_build_object('id',i.id)::text FROM app_credentials t JOIN app_installations i ON i.id=t.installation_id JOIN apps a ON a.id=i.app_id WHERE t.id=$1 AND t.revoked_at IS NULL AND t.expires_at>now() AND i.active AND a.status='installed' AND i.manifest::jsonb->>'transport'='websocket'",&[&credential]).await?;
    let installation = v["id"].as_str().unwrap().to_owned();
    tx.commit().await?;
    Ok(upgrade
        .max_message_size(64 * 1024)
        .on_upgrade(move |ws| run_socket(s, ws, installation, credential)))
}
pub(crate) async fn active(s: &AppState, credential: &str) -> Result<bool, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(one(&mut tx,"SELECT json_build_object('ok',EXISTS(SELECT 1 FROM app_credentials t JOIN app_installations i ON i.id=t.installation_id JOIN apps a ON a.id=i.app_id WHERE t.id=$1 AND t.revoked_at IS NULL AND t.expires_at>now() AND i.active AND a.status='installed'))::text",&[credential]).await?["ok"]==true)
}
pub async fn claim(s: &AppState, installation: &str) -> Result<Option<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let lease = uuid::Uuid::new_v4().to_string();
    let rows=query(&mut tx,"WITH candidate AS(SELECT d.id FROM app_deliveries d JOIN app_installations i ON i.id=d.installation_id JOIN apps a ON a.id=i.app_id WHERE i.id=$1 AND ((i.active AND a.status='installed') OR (d.event_type='app.uninstalled' AND a.status='disabled')) AND d.acked_at IS NULL AND d.available_at<=now() AND d.expires_at>now() AND (d.lease_until IS NULL OR d.lease_until<now()) AND (d.channel_id IS NULL OR EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.channel_id=d.channel_id AND cu.user_id=a.bot_user_id AND (c.deleted_at IS NULL OR d.event_type='channel.deleted'))) ORDER BY d.created_at,d.id FOR UPDATE OF d SKIP LOCKED LIMIT 1) UPDATE app_deliveries d SET attempts=attempts+1,lease_until=now()+interval '3 seconds',lease_token=$2,available_at=now()+make_interval(secs=>LEAST(3600,power(2,LEAST(attempts+1,12))::integer)) FROM candidate c WHERE d.id=c.id RETURNING json_build_object('delivery_id',d.id,'ack_token',d.lease_token,'event',d.payload::jsonb,'attempt',d.attempts)::text",&[installation,&lease]).await?;
    tx.commit().await?;
    let value = rows.into_iter().next();
    if let Some(event) = &value {
        tracing::info!(
            delivery_id = event["delivery_id"].as_str().unwrap_or(""),
            event_id = event["event"]["id"].as_str().unwrap_or(""),
            attempt = event["attempt"].as_u64().unwrap_or(0),
            "App event claimed"
        );
    }
    Ok(value)
}
async fn ack(s: &AppState, installation: &str, id: &str, lease: &str) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE app_deliveries SET acked_at=now(),lease_until=NULL,last_error=NULL WHERE id=$1 AND installation_id=$2 AND lease_token=$3 AND acked_at IS NULL").bind(id).bind(installation).bind(lease).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}
async fn run_socket(s: AppState, mut ws: WebSocket, installation: String, credential: String) {
    let mut ticks = tokio::time::interval(Duration::from_millis(300));
    loop {
        tokio::select! {
        _=ticks.tick()=>{
         if !active(&s,&credential).await.unwrap_or(false){terminal_socket(&s,&mut ws,&installation).await;break;}
         match claim(&s,&installation).await{
          Ok(Some(event))=>{
           if ws.send(Message::Text(event.to_string().into())).await.is_err(){break;}
          },Ok(None)=>{},Err(_)=>break
         }
        },
        frame=ws.recv()=>{match frame{
         Some(Ok(Message::Text(text)))=>{
          if !active(&s,&credential).await.unwrap_or(false){break;}
          if let Ok(v)=serde_json::from_str::<Value>(&text)&& let(Some(id),Some(token))=(v["ack"].as_str(),v["ack_token"].as_str())&& ack(&s,&installation,id,token).await.is_err(){break;}
         },Some(Ok(Message::Ping(data)))=>{if ws.send(Message::Pong(data)).await.is_err(){break;}},Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>{}
        }}
        }
    }
    let _ = ws.close().await;
}

async fn terminal_socket(s: &AppState, ws: &mut WebSocket, installation: &str) {
    let mut db = s.db.clone();
    let Ok(mut tx) = db.transaction().await else {
        return;
    };
    let disabled=one(&mut tx,"SELECT json_build_object('ok',EXISTS(SELECT 1 FROM apps a JOIN app_installations i ON i.app_id=a.id WHERE i.id=$1 AND a.status='disabled'))::text",&[installation]).await.ok().is_some_and(|v|v["ok"]==true);
    drop(tx);
    if !disabled {
        return;
    }
    if let Ok(Some(event)) = claim(s, installation).await {
        if ws
            .send(Message::Text(event.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
        if let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(3), ws.recv()).await
            && let Ok(v) = serde_json::from_str::<Value>(&text)
            && v["ack"] == event["delivery_id"]
            && v["ack_token"] == event["ack_token"]
        {
            let _ = ack(
                s,
                installation,
                event["delivery_id"].as_str().unwrap(),
                event["ack_token"].as_str().unwrap(),
            )
            .await;
        }
    }
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_multicast()
                && !v.is_unspecified()
                && !v.is_broadcast()
                && o[0] != 0
                && o[0] < 224
                && !(o[0] == 100 && (64..128).contains(&o[1]))
                && !(o[0] == 192 && o[1] == 0)
                && !(o[0] == 198 && (o[1] == 18 || o[1] == 19))
                && !v.is_documentation()
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let seg = v.segments();
            (seg[0] & 0xe000) == 0x2000
                && !(seg[0] == 0x2001 && (seg[1] <= 0x01ff || seg[1] == 0x0db8))
        }
    }
}
async fn deliver(url: &str, signing: &str, event: &Value) -> Result<(), AppError> {
    validate_https(url)?;
    let url = reqwest::Url::parse(url).map_err(|e| AppError::Internal(e.into()))?;
    let host = url.host_str().unwrap();
    let port = url.port_or_known_default().unwrap();
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AppError::BadGateway("Event host lookup failed".into()))?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(AppError::BadRequest(
            "Event endpoint must resolve to public addresses".into(),
        ));
    }
    // Pin DNS and disable redirects/proxies to prevent validation-to-request rebinding.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| AppError::Internal(e.into()))?;
    let timestamp = jiff::Timestamp::now().as_second().to_string();
    let body = event.to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(signing.as_bytes()).unwrap();
    mac.update(format!("v1:{timestamp}:{body}").as_bytes());
    let signature = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("x-zoo-timestamp", timestamp)
        .header("x-zoo-signature", format!("v1={signature}"))
        .body(body)
        .send()
        .await
        .map_err(|_| AppError::BadGateway("Event delivery failed or timed out".into()))?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway(format!(
            "Event endpoint returned {}",
            response.status().as_u16()
        )));
    }
    Ok(())
}
pub async fn worker(s: AppState) {
    let mut ticks = tokio::time::interval(Duration::from_millis(500));
    let mut passes = 0u32;
    loop {
        ticks.tick().await;
        passes = passes.wrapping_add(1);
        if passes.is_multiple_of(120)
            && let Err(e) = cleanup(&s).await
        {
            tracing::warn!(error=%e,"App maintenance failed");
        }
        if let Err(e) = http_pass(&s).await {
            tracing::warn!(error=%e,"App delivery pass failed");
        }
    }
}
async fn http_pass(s: &AppState) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let installs=query(&mut tx,"SELECT json_build_object('id',i.id,'url',i.manifest::jsonb->>'event_url','secret',i.signing_secret)::text FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE (i.active AND a.status='installed' OR a.status='disabled' AND EXISTS(SELECT 1 FROM app_deliveries terminal WHERE terminal.installation_id=i.id AND terminal.event_type='app.uninstalled' AND terminal.acked_at IS NULL AND terminal.expires_at>now())) AND i.manifest::jsonb->>'transport'='http' AND EXISTS(SELECT 1 FROM app_deliveries d WHERE d.installation_id=i.id AND d.acked_at IS NULL AND d.available_at<=now() AND d.expires_at>now()) LIMIT 100",&[]).await?;
    drop(tx);
    let results = futures_util::stream::iter(installs)
        .map(|i| http_install(s, i))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
    for result in results {
        result?;
    }

    Ok(())
}

async fn http_install(s: &AppState, i: Value) -> Result<(), AppError> {
    let id = i["id"].as_str().unwrap();
    if let Some(event) = claim(s, id).await? {
        match tokio::time::timeout(
            Duration::from_secs(3),
            deliver(
                i["url"].as_str().unwrap_or(""),
                i["secret"].as_str().unwrap(),
                &event,
            ),
        )
        .await
        .unwrap_or_else(|_| Err(AppError::BadGateway("Event delivery timed out".into())))
        {
            Ok(()) => {
                ack(
                    s,
                    id,
                    event["delivery_id"].as_str().unwrap(),
                    event["ack_token"].as_str().unwrap(),
                )
                .await?
            }
            Err(e) => {
                let mut db = s.db.clone();
                let mut tx = db.transaction().await?;
                toasty::sql::statement(
                    "UPDATE app_deliveries SET last_error=$2 WHERE id=$1 AND lease_token=$3",
                )
                .bind(event["delivery_id"].as_str().unwrap())
                .bind(e.to_string())
                .bind(event["ack_token"].as_str().unwrap())
                .exec(&mut tx)
                .await?;
                tx.commit().await?;
            }
        }
    }

    Ok(())
}

async fn cleanup(s: &AppState) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    for sql in [
        "DELETE FROM app_rate_limits WHERE window_start<now()-interval '1 hour'",
        "DELETE FROM app_socket_tickets WHERE expires_at<now()-interval '1 day'",
        "DELETE FROM app_ui WHERE expires_at<now()",
        "DELETE FROM app_responses WHERE expires_at<now()",
        "DELETE FROM app_oauth_codes WHERE expires_at<now()-interval '1 day'",
        "DELETE FROM app_deliveries WHERE created_at<now()-interval '30 days'",
    ] {
        toasty::sql::statement(sql).exec(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_nonpublic_destinations() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip(IpAddr::V4(std::net::Ipv4Addr::new(8, 8, 8, 8))));
    }
}
