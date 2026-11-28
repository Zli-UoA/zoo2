//! 本文のリンクが指すメッセージの取得（docs/request-design.md の「メッセージリンクの埋め込み」）。
//!
//! リンク先のチャンネルごとに 1 本の `messages` シェイプを `log=changes_only` で購読し、行は
//! `id = $1` の subset snapshot で 1 件ずつ取る。取得した行の編集・削除は live で届く。
//! チャンネルの全件は同期しない。

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{Instant, sleep_until, timeout},
};

use super::{Change, Row, ShapeId, ShapeStream};
use crate::http::{Credentials, HttpError};

#[derive(Debug)]
pub enum LinkedEvent {
    /// 取得が終わった ID ごとの行。その ID のメッセージがなければ `None`。
    Rows {
        channel_id: String,
        rows: Vec<(String, Option<Row>)>,
    },
    /// 暗号文のある行が届いた（取得を頼まれていない行を含む）。購読ごとに 1 回で、その行より先に送る。
    Ciphertext { channel_id: String },
    /// 再試行しても取得できない失敗（4xx）。購読は止まる。
    Failed {
        channel_id: String,
        message_ids: Vec<String>,
    },
}

impl LinkedEvent {
    pub fn channel_id(&self) -> &str {
        match self {
            LinkedEvent::Rows { channel_id, .. }
            | LinkedEvent::Ciphertext { channel_id }
            | LinkedEvent::Failed { channel_id, .. } => channel_id,
        }
    }
}

/// 行に暗号文があるか。
pub fn has_ciphertext(row: &Row) -> bool {
    row.get("ciphertext").is_some_and(|value| !value.is_null())
}

/// 失敗したあと、次に試すまでの間隔。失敗のたびに倍にする。
const FIRST_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// 取得の待ち時間の上限。
#[derive(Clone, Copy)]
struct Limits {
    /// subset の取得 1 回。取得中は、同じチャンネルのほかの取得と live の読み取りが待つので打ち切る。
    subset: Duration,
    /// 取得を打ち切ったあと、live の読み取りを待つ時間。この間は次の取得を始めない。
    live_window: Duration,
}

const LIMITS: Limits = Limits {
    subset: Duration::from_secs(15),
    live_window: Duration::from_secs(5),
};

/// 取得を頼まれているメッセージと、その取得の状態。
#[derive(Default)]
struct Wanted {
    /// 先に取る順。
    order: Vec<String>,
    ids: HashSet<String>,
    fetched: HashSet<String>,
    /// 取得に失敗した ID の、次に試す時刻と、そこまでの間隔。
    retries: HashMap<String, (Instant, Duration)>,
    /// シェイプのキーからメッセージの ID を引く（削除はキーだけが届く）。
    keys: HashMap<String, String>,
}

impl Wanted {
    /// 頼まれている ID を入れ替える。外れた ID は、もう取らず、取り直しもしない。
    fn replace(&mut self, order: Vec<String>) {
        self.ids = order.iter().cloned().collect();
        self.order = order;
        let ids = &self.ids;
        self.fetched.retain(|id| ids.contains(id));
        self.retries.retain(|id, _| ids.contains(id));
        self.keys.retain(|_, id| ids.contains(id));
    }

    /// 次に取る ID。いま取るものがなければ、再試行を待っている中でいちばん早い時刻。
    fn next(&self, now: Instant) -> Result<&str, Option<Instant>> {
        let mut soonest: Option<Instant> = None;
        for id in self.order.iter().filter(|id| !self.fetched.contains(*id)) {
            match self.retries.get(id) {
                Some((at, _)) if *at > now => {
                    soonest = Some(soonest.map_or(*at, |soonest| soonest.min(*at)));
                }
                _ => return Ok(id),
            }
        }
        Err(soonest)
    }

    fn retry_later(&mut self, id: String) {
        let wait = self
            .retries
            .get(&id)
            .map_or(FIRST_BACKOFF, |(_, wait)| (*wait * 2).min(MAX_BACKOFF));
        self.retries.insert(id, (Instant::now() + wait, wait));
    }

    /// 取り直し。取得済みの ID をもう一度取る。
    fn refetch(&mut self) {
        self.fetched.clear();
        self.keys.clear();
    }
}

/// 再試行しても取得できない失敗か。
fn is_fatal(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<HttpError>() {
        Some(HttpError::Unauthorized) => true,
        Some(HttpError::Status { status, .. }) => {
            status.is_client_error() && status.as_u16() != 429
        }
        None => false,
    }
}

/// `requests` で頼まれたメッセージを取り、変化を受け取り続けるタスクを起動する。`requests` には、いま
/// 必要な ID をすべて、先に取る順で送る。通信エラーと 5xx は間隔を延ばして再試行し（再試行を待つ間も、
/// ほかの ID の取得と live の読み取りは進める）、4xx では `Failed` を送って止まる。`requests` を閉じるか、
/// `events` が `false` を返すと止まる。
pub fn spawn_linked(
    http: reqwest::Client,
    api_base_url: &str,
    credentials: Credentials,
    channel_id: String,
    requests: mpsc::UnboundedReceiver<Vec<String>>,
    events: impl Fn(LinkedEvent) -> bool + Send + 'static,
) -> JoinHandle<()> {
    let (path, params) = ShapeId::Messages(channel_id.clone()).path_and_params();
    let url = format!("{}/api/shapes/{path}", api_base_url.trim_end_matches('/'));
    let stream = ShapeStream::changes_only(http, url, params, credentials);
    tokio::spawn(run(stream, channel_id, requests, events, LIMITS))
}

async fn run(
    mut stream: ShapeStream,
    channel_id: String,
    mut requests: mpsc::UnboundedReceiver<Vec<String>>,
    events: impl Fn(LinkedEvent) -> bool + Send + 'static,
    limits: Limits,
) {
    let mut wanted = Wanted::default();
    // live の読み取りに失敗したら、この時刻まで読まない。
    let mut live_at = Instant::now();
    let mut live_backoff = FIRST_BACKOFF;
    // 取得を打ち切ったら、live を 1 回読むか、この時刻になるまで、次の取得を始めない。
    let mut live_only: Option<Instant> = None;
    let mut ciphertext_reported = false;
    loop {
        while let Ok(order) = requests.try_recv() {
            wanted.replace(order);
        }
        let now = Instant::now();
        live_only = live_only.filter(|until| *until > now);
        let started = stream.is_started();
        // 次に取る ID か、live を読みながら待つ時刻。
        let (pending, wake_at) = match live_only {
            _ if !started => (None, None),
            Some(until) => (None, Some(until)),
            None => match wanted.next(now) {
                Ok(id) => (Some(id.to_owned()), None),
                Err(at) => (None, at),
            },
        };
        // `ciphertext` は、届いた中に暗号文の行があるか。
        let (ciphertext, result) = if let Some(id) = pending {
            let values = [id.as_str()];
            let subset = stream.subset("id = $1", &values, "id", 1);
            match timeout(limits.subset, subset).await {
                Ok(Ok(Some(rows))) => {
                    wanted.fetched.insert(id.clone());
                    wanted.retries.remove(&id);
                    let ciphertext = rows.iter().any(has_ciphertext);
                    let row = rows
                        .into_iter()
                        .find(|row| text(row, "id") == Some(id.as_str()));
                    (ciphertext, Ok(vec![(id, row)]))
                }
                Ok(Ok(None)) => {
                    wanted.refetch();
                    continue;
                }
                Ok(Err(err)) if is_fatal(&err) => (false, Err(err)),
                Ok(Err(_)) => {
                    wanted.retry_later(id);
                    continue;
                }
                Err(_) => {
                    wanted.retry_later(id);
                    live_only = Some(Instant::now() + limits.live_window);
                    continue;
                }
            }
        } else {
            let batch = {
                let live = async {
                    sleep_until(live_at).await;
                    stream.next_batch().await
                };
                tokio::pin!(live);
                loop {
                    tokio::select! {
                        order = requests.recv() => {
                            let Some(order) = order else { return };
                            wanted.replace(order);
                            // すぐに取るものがなければ、live の読み取りを続ける。
                            if started && live_only.is_none() && wanted.next(Instant::now()).is_ok() {
                                break None;
                            }
                        }
                        // 条件が偽でも式は評価されるので、予定がないときの時刻も用意する。
                        () = sleep_until(wake_at.unwrap_or(now)), if wake_at.is_some() => break None,
                        batch = &mut live => break Some(batch),
                    }
                }
            };
            let Some(batch) = batch else { continue };
            live_only = None;
            match batch {
                Ok(batch) => {
                    live_backoff = FIRST_BACKOFF;
                    if batch.reset {
                        wanted.refetch();
                    }
                    let ciphertext = batch.changes.iter().any(
                        |change| matches!(change, Change::Upsert { row, .. } if has_ciphertext(row)),
                    );
                    let rows = batch
                        .changes
                        .into_iter()
                        .filter_map(|change| match change {
                            Change::Upsert { key, row } => {
                                let id = wanted
                                    .keys
                                    .get(&key)
                                    .cloned()
                                    .or_else(|| text(&row, "id").map(str::to_owned))?;
                                // 取っていない行の変化は、列が欠けているので使わない。
                                wanted.fetched.contains(&id).then(|| {
                                    wanted.keys.insert(key, id.clone());
                                    (id, Some(row))
                                })
                            }
                            Change::Delete { key } => {
                                wanted.keys.get(&key).map(|id| (id.clone(), None))
                            }
                        })
                        .collect();
                    (ciphertext, Ok(rows))
                }
                Err(err) if is_fatal(&err) => (false, Err(err)),
                Err(_) => {
                    live_at = Instant::now() + live_backoff;
                    live_backoff = (live_backoff * 2).min(MAX_BACKOFF);
                    continue;
                }
            }
        };
        let Ok(rows) = result else {
            while let Ok(order) = requests.try_recv() {
                wanted.replace(order);
            }
            let _ = events(LinkedEvent::Failed {
                channel_id,
                message_ids: wanted.order,
            });
            return;
        };
        for (id, row) in &rows {
            // subset で取った行は、キーを覚えるために ID から引けるようにする。
            if row.is_some()
                && let Some(key) = stream.key_of(id)
            {
                wanted.keys.insert(key, id.clone());
            }
        }
        // チャンネルの新しい投稿などは持たない。
        stream.retain(|row| text(row, "id").is_some_and(|id| wanted.ids.contains(id)));
        if ciphertext && !ciphertext_reported {
            ciphertext_reported = true;
            if !events(LinkedEvent::Ciphertext {
                channel_id: channel_id.clone(),
            }) {
                return;
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

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn spawn_with(
        server: &MockServer,
        limits: Limits,
    ) -> (
        mpsc::UnboundedSender<Vec<String>>,
        mpsc::UnboundedReceiver<LinkedEvent>,
        JoinHandle<()>,
    ) {
        let (requests, requests_rx) = mpsc::unbounded_channel();
        let (events, events_rx) = mpsc::unbounded_channel();
        let stream = ShapeStream::changes_only(
            reqwest::Client::new(),
            format!("{}/api/shapes/messages", server.uri()),
            vec![("channel_id".into(), "c1".into())],
            Credentials::Dev("u1".into()),
        );
        let task = tokio::spawn(run(
            stream,
            "c1".into(),
            requests_rx,
            move |event| events.send(event).is_ok(),
            limits,
        ));
        (requests, events_rx, task)
    }

    /// 開始の応答と、取り直しのない live の待ち受けを用意する。
    async fn mount_start(server: &MockServer) {
        Mock::given(method("GET"))
            .and(query_param("offset", "now"))
            .and(query_param_is_missing("subset__where"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!([
                { "headers": { "control": "up-to-date" } }
            ]))))
            .mount(server)
            .await;
    }

    async fn mount_live_once(server: &MockServer, delay: Duration, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(
                ResponseTemplate::new(200)
                    .set_delay(delay)
                    .set_body_json(body),
            ))
            .up_to_n_times(1)
            .mount(server)
            .await;
    }

    async fn mount_idle_live(server: &MockServer) {
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(
                ResponseTemplate::new(204).set_delay(Duration::from_secs(30)),
            ))
            .mount(server)
            .await;
    }

    fn headers(response: ResponseTemplate) -> ResponseTemplate {
        response
            .insert_header("electric-handle", "h1")
            .insert_header("electric-offset", "0_0")
            .insert_header("electric-cursor", "1")
    }

    fn spawn(
        server: &MockServer,
    ) -> (
        mpsc::UnboundedSender<Vec<String>>,
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
        let (requests, mut events, task) = spawn(&server);
        requests.send(ids(&["m1"])).unwrap();
        requests.send(ids(&["m1"])).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("行が届く");
        };
        assert_eq!(rows[0].0, "m1");
        assert_eq!(rows[0].1.as_ref().unwrap()["content"], "本文");
        // 待ち受け中に増えたリンクも取る。同じ ID は取り直さない。
        requests.send(ids(&["m2", "m1"])).unwrap();
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
    async fn a_failing_message_does_not_block_the_others() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("offset", "now"))
            .and(query_param_is_missing("subset__where"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!([
                { "headers": { "control": "up-to-date" } }
            ]))))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("subset__params", r#"{"1":"m1"}"#))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("subset__params", r#"{"1":"m2"}"#))
            .respond_with(headers(
                ResponseTemplate::new(200).set_body_json(subset("m2")),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!([
                { "key": "\"public\".\"messages\"/\"m2\"", "headers": { "operation": "delete" } },
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
        let (requests, mut events, task) = spawn(&server);
        // `m1` は失敗し続ける。`m2` の取得と、そのあとの削除は届く。
        requests.send(ids(&["m1", "m2"])).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("行が届く");
        };
        assert_eq!(rows[0].0, "m2");
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("削除が届く");
        };
        assert_eq!(rows, [("m2".to_owned(), None)]);
        task.abort();
    }

    #[tokio::test]
    async fn reports_a_rejected_request_and_stops() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let (requests, mut events, task) = spawn(&server);
        requests.send(ids(&["m1"])).unwrap();
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
        let (requests, mut events, task) = spawn(&server);
        requests.send(ids(&["m1"])).unwrap();
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

    #[tokio::test]
    async fn a_stalled_fetch_does_not_block_the_others() {
        let server = MockServer::start().await;
        mount_start(&server).await;
        Mock::given(method("GET"))
            .and(query_param("subset__params", r#"{"1":"m1"}"#))
            .respond_with(headers(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(30))
                    .set_body_json(subset("m1")),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("subset__params", r#"{"1":"m2"}"#))
            .respond_with(headers(
                ResponseTemplate::new(200).set_body_json(subset("m2")),
            ))
            .mount(&server)
            .await;
        // live は毎回 `m2` の削除を返す。`m2` を取るまでは使われず、取ったあとの読み取りで届く。
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(50))
                    .set_body_json(json!([
                        { "key": "\"public\".\"messages\"/\"m2\"", "headers": { "operation": "delete" } },
                        { "headers": { "control": "up-to-date" } }
                    ])),
            ))
            .mount(&server)
            .await;
        let limits = Limits {
            subset: Duration::from_secs(1),
            live_window: Duration::from_secs(1),
        };
        let (requests, mut events, task) = spawn_with(&server, limits);
        requests.send(ids(&["m1", "m2"])).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("行が届く");
        };
        assert_eq!(rows[0].0, "m2");
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("削除が届く");
        };
        assert_eq!(rows, [("m2".to_owned(), None)]);
        task.abort();
    }

    #[tokio::test]
    async fn released_messages_are_not_fetched_again() {
        let server = MockServer::start().await;
        mount_start(&server).await;
        for (id, times) in [("m1", 1), ("m2", 2)] {
            Mock::given(method("GET"))
                .and(query_param("subset__params", format!(r#"{{"1":"{id}"}}"#)))
                .respond_with(headers(
                    ResponseTemplate::new(200).set_body_json(subset(id)),
                ))
                .expect(times)
                .mount(&server)
                .await;
        }
        // 取り直しは、`m1` を外したあとに届く。
        mount_live_once(
            &server,
            Duration::from_secs(1),
            json!([{ "headers": { "control": "must-refetch" } }]),
        )
        .await;
        mount_idle_live(&server).await;
        let (requests, mut events, task) = spawn(&server);
        requests.send(ids(&["m1", "m2"])).unwrap();
        for id in ["m1", "m2"] {
            let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
                panic!("行が届く");
            };
            assert_eq!(rows[0].0, id);
        }
        requests.send(ids(&["m2"])).unwrap();
        let LinkedEvent::Rows { rows, .. } = next(&mut events).await else {
            panic!("取り直した行が届く");
        };
        assert_eq!(rows[0].0, "m2");
        task.abort();
        server.verify().await;
    }

    #[tokio::test]
    async fn ciphertext_in_any_row_is_reported_once() {
        let server = MockServer::start().await;
        mount_start(&server).await;
        let secret = |id: &str| {
            json!({ "key": id, "value": { "id": id, "channel_id": "c1", "ciphertext": "{}" },
                    "headers": { "operation": "insert" } })
        };
        // 取得を頼んでいない行の暗号文も知らせる。
        mount_live_once(
            &server,
            Duration::ZERO,
            json!([secret("x1"), { "headers": { "control": "up-to-date" } }]),
        )
        .await;
        mount_live_once(
            &server,
            Duration::ZERO,
            json!([secret("x2"), { "headers": { "control": "up-to-date" } }]),
        )
        .await;
        mount_idle_live(&server).await;
        Mock::given(method("GET"))
            .and(query_param("subset__params", r#"{"1":"m1"}"#))
            .respond_with(headers(
                ResponseTemplate::new(200).set_body_json(subset("m1")),
            ))
            .mount(&server)
            .await;
        let (requests, mut events, task) = spawn(&server);
        let LinkedEvent::Ciphertext { channel_id } = next(&mut events).await else {
            panic!("暗号文を知らせる");
        };
        assert_eq!(channel_id, "c1");
        requests.send(ids(&["m1"])).unwrap();
        assert!(matches!(next(&mut events).await, LinkedEvent::Rows { .. }));
        task.abort();
    }
}
