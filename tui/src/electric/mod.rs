pub mod rows;
pub mod shape;

use std::time::Duration;

use tokio::{sync::mpsc, task::JoinHandle};

use crate::http::{Credentials, HttpError};
pub use shape::{Batch, Change, Row, ShapeStream};

/// backend のシェイププロキシ (`/api/shapes/*`) のうち、TUI が購読するもの。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShapeId {
    Channels,
    Users,
    ChannelUsers,
    Messages(String),
    Reactions(String),
}

impl ShapeId {
    fn path_and_params(&self) -> (&'static str, Vec<(String, String)>) {
        let channel = |id: &String| vec![("channel_id".to_string(), id.clone())];
        match self {
            ShapeId::Channels => ("channels", Vec::new()),
            ShapeId::Users => ("users", Vec::new()),
            ShapeId::ChannelUsers => ("channel_users", Vec::new()),
            ShapeId::Messages(id) => ("messages", channel(id)),
            ShapeId::Reactions(id) => ("message_reactions", channel(id)),
        }
    }
}

#[derive(Debug)]
pub enum ShapeEvent {
    Batch(ShapeId, Batch),
    /// 回復できないエラー。購読は止まる。
    Failed(ShapeId, String),
}

/// シェイプを購読するタスクを起動する。通信エラーは間隔を延ばしながら再試行し、
/// 401 や 403/404 (権限がない) では止まって `Failed` を送る。
pub fn spawn(
    http: reqwest::Client,
    api_base_url: &str,
    credentials: Credentials,
    id: ShapeId,
    events: mpsc::UnboundedSender<ShapeEvent>,
) -> JoinHandle<()> {
    let (path, params) = id.path_and_params();
    let url = format!("{}/api/shapes/{path}", api_base_url.trim_end_matches('/'));
    let mut stream = ShapeStream::new(http, url, params, credentials);
    tokio::spawn(async move {
        let mut backoff = Duration::from_millis(500);
        loop {
            match stream.next_batch().await {
                Ok(batch) => {
                    backoff = Duration::from_millis(500);
                    if events.send(ShapeEvent::Batch(id.clone(), batch)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let fatal = match err.downcast_ref::<HttpError>() {
                        Some(HttpError::Unauthorized) => true,
                        Some(HttpError::Status { status, .. }) => status.is_client_error(),
                        None => false,
                    };
                    if fatal {
                        let _ = events.send(ShapeEvent::Failed(id, err.to_string()));
                        return;
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    })
}
