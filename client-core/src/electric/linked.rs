//! 本文のリンクが指すメッセージの取得（docs/request-design.md の「メッセージリンクの埋め込み」）。
//!
//! リンク先のチャンネルごとに 1 本の `messages` シェイプを `log=changes_only` で購読し、行は
//! `id = $1` の subset snapshot で 1 件ずつ取る。取得した行の編集・削除は live で届く。
//! チャンネルの全件は同期しない。

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::Duration,
};

use tokio::{sync::mpsc, task::JoinHandle};

use super::{Change, Row, ShapeId, ShapeStream};
use crate::http::{Credentials, HttpError};

#[derive(Debug)]
pub enum LinkedEvent {
    /// 取得が終わった ID ごとの行。その ID のメッセージがなければ `None`。
    Rows {
        channel_id: String,
        rows: Vec<(String, Option<Row>)>,
    },
    /// 再試行しても取得できない失敗（4xx）。購読は止まる。
    Failed {
        channel_id: String,
        message_ids: Vec<String>,
    },
}

/// `ids` に送られたメッセージを取り続けるタスクを起動する。通信エラーと 5xx は間隔を延ばして
/// 再試行し、4xx では `Failed` を送って止まる。`ids` を閉じるか、`events` が `false` を返すと止まる。
pub fn spawn_linked(
    http: reqwest::Client,
    api_base_url: &str,
    credentials: Credentials,
    channel_id: String,
    mut ids: mpsc::UnboundedReceiver<String>,
    events: impl Fn(LinkedEvent) -> bool + Send + 'static,
) -> JoinHandle<()> {
    let (path, params) = ShapeId::Messages(channel_id.clone()).path_and_params();
    let url = format!("{}/api/shapes/{path}", api_base_url.trim_end_matches('/'));
    let mut stream = ShapeStream::changes_only(http, url, params, credentials);
    tokio::spawn(async move {
        let mut wanted = BTreeSet::new();
        let mut fetched = HashSet::new();
        // シェイプのキーからメッセージの ID を引く（削除はキーだけが届く）。
        let mut keys: HashMap<String, String> = HashMap::new();
        let mut backoff = Duration::from_millis(500);
        loop {
            while let Ok(id) = ids.try_recv() {
                wanted.insert(id);
            }
            let pending: Option<String> = wanted.iter().find(|id| !fetched.contains(*id)).cloned();
            let result = if let (true, Some(id)) = (stream.is_started(), pending) {
                stream
                    .subset("id = $1", &[&id], "id", 1)
                    .await
                    .map(|rows| match rows {
                        Some(rows) => {
                            fetched.insert(id.clone());
                            let row = rows
                                .into_iter()
                                .find(|row| text(row, "id") == Some(id.as_str()));
                            vec![(id, row)]
                        }
                        // 取り直し。取得済みの ID をもう一度取る。
                        None => {
                            fetched.clear();
                            Vec::new()
                        }
                    })
            } else {
                let batch = tokio::select! {
                    id = ids.recv() => {
                        match id {
                            Some(id) => {
                                wanted.insert(id);
                                continue;
                            }
                            None => return,
                        }
                    }
                    batch = stream.next_batch() => batch,
                };
                batch.map(|batch| {
                    if batch.reset {
                        fetched.clear();
                        keys.clear();
                    }
                    batch
                        .changes
                        .into_iter()
                        .filter_map(|change| match change {
                            Change::Upsert { key, row } => {
                                let id = keys
                                    .get(&key)
                                    .cloned()
                                    .or_else(|| text(&row, "id").map(str::to_owned))?;
                                // 取っていない行の変化は、列が欠けているので使わない。
                                fetched.contains(&id).then(|| {
                                    keys.insert(key, id.clone());
                                    (id, Some(row))
                                })
                            }
                            Change::Delete { key } => keys.get(&key).map(|id| (id.clone(), None)),
                        })
                        .collect()
                })
            };
            match result {
                Ok(rows) => {
                    backoff = Duration::from_millis(500);
                    for (id, row) in &rows {
                        // subset で取った行は、キーを覚えるために ID から引けるようにする。
                        if row.is_some()
                            && let Some(key) = stream.key_of(id)
                        {
                            keys.insert(key, id.clone());
                        }
                    }
                    if !rows.is_empty()
                        && !events(LinkedEvent::Rows {
                            channel_id: channel_id.clone(),
                            rows,
                        })
                    {
                        return;
                    }
                }
                Err(err) => {
                    let fatal = match err.downcast_ref::<HttpError>() {
                        Some(HttpError::Unauthorized) => true,
                        Some(HttpError::Status { status, .. }) => {
                            status.is_client_error() && status.as_u16() != 429
                        }
                        None => false,
                    };
                    if fatal {
                        while let Ok(id) = ids.try_recv() {
                            wanted.insert(id);
                        }
                        let _ = events(LinkedEvent::Failed {
                            channel_id,
                            message_ids: wanted.into_iter().collect(),
                        });
                        return;
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    })
}

fn text<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    row.get(column)?.as_str()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;

    fn subset(id: &str) -> serde_json::Value {
        json!({ "metadata": {}, "data": [{
            "key": format!("\"public\".\"messages\"/\"{id}\""),
            "value": { "id": id, "channel_id": "c1", "content": "本文" },
            "headers": { "operation": "insert" },
        }] })
    }

    async fn next(events: &mut mpsc::UnboundedReceiver<LinkedEvent>) -> LinkedEvent {
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("時間内に届く")
            .expect("タスクが動いている")
    }

    fn spawn(
        server: &MockServer,
    ) -> (
        mpsc::UnboundedSender<String>,
        mpsc::UnboundedReceiver<LinkedEvent>,
        JoinHandle<()>,
    ) {
        let (ids, ids_rx) = mpsc::unbounded_channel();
        let (events, events_rx) = mpsc::unbounded_channel();
        let task = spawn_linked(
            reqwest::Client::new(),
            &server.uri(),
            Credentials::Dev("u1".into()),
            "c1".into(),
            ids_rx,
            move |event| events.send(event).is_ok(),
        );
        (ids, events_rx, task)
    }

    #[tokio::test]
    async fn fetches_each_linked_message_once_without_syncing_the_channel() {
        let server = MockServer::start().await;
        let headers = |response: ResponseTemplate| {
            response
                .insert_header("electric-handle", "h1")
                .insert_header("electric-offset", "0_0")
                .insert_header("electric-cursor", "1")
        };
        Mock::given(method("GET"))
            .and(path("/api/shapes/messages"))
            .and(query_param("offset", "now"))
            .and(query_param("log", "changes_only"))
            .and(query_param("channel_id", "c1"))
            .and(query_param_is_missing("subset__where"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!([
                { "headers": { "control": "up-to-date" } }
            ]))))
            .expect(1)
            .mount(&server)
            .await;
        for id in ["m1", "m2"] {
            Mock::given(method("GET"))
                .and(query_param("subset__where", "id = $1"))
                .and(query_param("subset__params", format!(r#"{{"1":"{id}"}}"#)))
                .and(query_param("log", "changes_only"))
                .and(query_param("handle", "h1"))
                .respond_with(headers(
                    ResponseTemplate::new(200).set_body_json(subset(id)),
                ))
                .expect(1)
                .mount(&server)
                .await;
        }
        // live の待ち受けは、返事を遅らせて続けさせる。
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(
                ResponseTemplate::new(204).set_delay(Duration::from_secs(30)),
            ))
            .mount(&server)
            .await;
        let (ids, mut events, task) = spawn(&server);
        ids.send("m1".into()).unwrap();
        ids.send("m1".into()).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("行が届く");
        };
        assert_eq!(rows[0].0, "m1");
        assert_eq!(rows[0].1.as_ref().unwrap()["content"], "本文");
        // 待ち受け中に増えたリンクも取る。同じ ID は取り直さない。
        ids.send("m1".into()).unwrap();
        ids.send("m2".into()).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("行が届く");
        };
        assert_eq!(rows[0].0, "m2");
        let requests: Vec<Request> = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|request| !request.url.query().unwrap_or("").contains("offset=-1")),
            "チャンネルの全件は同期しない"
        );
        task.abort();
        server.verify().await;
    }

    #[tokio::test]
    async fn reports_a_rejected_request_and_stops() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let (ids, mut events, task) = spawn(&server);
        ids.send("m1".into()).unwrap();
        let LinkedEvent::Failed { message_ids, .. } = next(&mut events).await else {
            panic!("失敗が届く");
        };
        assert_eq!(message_ids, ["m1"]);
        task.await.unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn delivers_deletions_of_fetched_messages() {
        let server = MockServer::start().await;
        let headers = |response: ResponseTemplate| {
            response
                .insert_header("electric-handle", "h1")
                .insert_header("electric-offset", "0_0")
                .insert_header("electric-cursor", "1")
        };
        Mock::given(method("GET"))
            .and(query_param("offset", "now"))
            .and(query_param_is_missing("subset__where"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!([
                { "headers": { "control": "up-to-date" } }
            ]))))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("subset__where", "id = $1"))
            .respond_with(headers(
                ResponseTemplate::new(200).set_body_json(subset("m1")),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!([
                { "key": "\"public\".\"messages\"/\"m1\"", "headers": { "operation": "delete" } },
                { "headers": { "control": "up-to-date" } }
            ]))))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(
                ResponseTemplate::new(204).set_delay(Duration::from_secs(30)),
            ))
            .mount(&server)
            .await;
        let (ids, mut events, task) = spawn(&server);
        ids.send("m1".into()).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("行が届く");
        };
        assert!(rows[0].1.is_some());
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("削除が届く");
        };
        assert_eq!(rows, [("m1".to_owned(), None)]);
        task.abort();
    }
}
