//! Content-free invalidation over authenticated WebSockets. REST always rechecks audience.
use crate::{auth::AuthUser, state::AppState};
use axum::{
    extract::{
        FromRequestParts, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderName, HeaderValue, Request},
    response::Response,
};
use futures_util::future::poll_fn;
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

#[derive(Clone, Default)]
pub struct HomeLive(Arc<OnceLock<broadcast::Sender<()>>>);

impl HomeLive {
    fn subscribe(&self, database_url: String) -> broadcast::Receiver<()> {
        self.0
            .get_or_init(|| {
                let (events, _) = broadcast::channel(16);
                let sender = events.clone();
                tokio::spawn(async move {
                    loop {
                        if let Ok((client, mut connection)) =
                            tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await
                        {
                            let notifications = sender.clone();
                            let task = tokio::spawn(async move {
                                while let Some(Ok(message)) =
                                    poll_fn(|cx| connection.poll_message(cx)).await
                                {
                                    if matches!(
                                        message,
                                        tokio_postgres::AsyncMessage::Notification(_)
                                    ) {
                                        let _ = notifications.send(());
                                    }
                                }
                            });
                            if client.batch_execute("LISTEN home_timeline").await.is_ok() {
                                // Refresh after reconnect too: changes may have happened while disconnected.
                                let _ = sender.send(());
                                let _ = task.await;
                            } else {
                                task.abort();
                            }
                        }
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                });
                events
            })
            .subscribe()
    }
}

pub async fn upgrade(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| serve(state, socket))
}

async fn serve(state: AppState, mut socket: WebSocket) {
    // Tokens are carried in the first frame, never in URLs, logs, or subprotocols.
    let Ok(Some(Ok(Message::Text(frame)))) =
        tokio::time::timeout(Duration::from_secs(10), socket.recv()).await
    else {
        return;
    };
    let Ok(headers) = serde_json::from_str::<std::collections::HashMap<String, String>>(&frame)
    else {
        return;
    };
    let (mut parts, _) = Request::new(()).into_parts();
    for name in ["authorization", "x-dev-user"] {
        if let Some(value) = headers
            .get(name)
            .and_then(|s| HeaderValue::from_str(s).ok())
        {
            parts.headers.insert(HeaderName::from_static(name), value);
        }
    }
    if AuthUser::from_request_parts(&mut parts, &state)
        .await
        .is_err()
    {
        return;
    }
    let mut events = state.home_live.subscribe(state.config.database_url.clone());
    if socket.send(Message::Text("refresh".into())).await.is_err() {
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    // Renew authentication regularly by reconnecting with a fresh token.
    let renewal = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(renewal);
    let mut last_received = Instant::now();
    loop {
        tokio::select! {
            _ = &mut renewal => break,
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => { last_received = Instant::now(); }
            },
            _ = heartbeat.tick() => {
                if last_received.elapsed() > Duration::from_secs(60) { break; }
                if socket.send(Message::Ping(vec![].into())).await.is_err() { break; }
            },
            _ = events.recv() => {
                if socket.send(Message::Text("refresh".into())).await.is_err() { break; }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}
