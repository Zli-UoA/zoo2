//! Electric v1 の HTTP シェイププロトコル。
//!
//! 1. `offset=-1` で初回取得し、`electric-handle` と `electric-offset` を追う。
//! 2. `up-to-date` を受けたら `live=true` と `electric-cursor` を付けたロングポーリングに移る。
//! 3. 409 か `must-refetch` を受けたら行を捨て、新しい handle で最初から取り直す。
//!
//! update は既定 (`replica=default`) で変更列と主キーだけが届くため、ここで行を保持して
//! マージし、呼び出し側には常に完全な行を渡す。
//!
//! `log=changes_only` の subset snapshot のあとの live は、取得前の位置から読む。snapshot に
//! 入っている変更が届いても、取得した行には重ねない（Electric のクライアントの `SnapshotTracker`
//! と同じ判定）。

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use reqwest::{StatusCode, header::HeaderMap};
use serde::Deserialize;
use serde_json::Value;

use crate::http::{Credentials, error_for_status};

pub use zoo2_client_types::shape::{Batch, Change, Row};

#[derive(Debug, Deserialize)]
struct Message {
    headers: MessageHeaders,
    key: Option<String>,
    value: Option<Row>,
}

#[derive(Debug, Deserialize)]
struct Subset {
    #[serde(default)]
    metadata: SubsetMetadata,
    data: Vec<Message>,
}

/// snapshot を取った時点の Postgres の状態。数値は文字列で届く。
#[derive(Debug, Default, Deserialize)]
struct SubsetMetadata {
    xmin: Option<Value>,
    xmax: Option<Value>,
    xip_list: Option<Vec<Value>>,
    database_lsn: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct MessageHeaders {
    operation: Option<String>,
    control: Option<String>,
    /// 変更を行ったトランザクション。
    txids: Option<Vec<Value>>,
    /// 変更の WAL の位置。
    lsn: Option<Value>,
    /// `up-to-date` までに読み終えた WAL の位置。
    global_last_seen_lsn: Option<Value>,
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

/// 取得した subset snapshot のうち、live がまだ追いついていないもの。
struct Snapshot {
    xmin: u64,
    xmax: u64,
    /// 取った時点で終わっていないトランザクション。
    in_progress: Vec<u64>,
    database_lsn: Option<u64>,
    /// snapshot に入っていた行のキー。
    keys: HashSet<String>,
}

impl Snapshot {
    fn new(metadata: &SubsetMetadata, keys: HashSet<String>) -> Option<Self> {
        Some(Self {
            xmin: number(metadata.xmin.as_ref()?)?,
            xmax: number(metadata.xmax.as_ref()?)?,
            in_progress: metadata
                .xip_list
                .iter()
                .flatten()
                .filter_map(number)
                .collect(),
            database_lsn: metadata.database_lsn.as_ref().and_then(number),
            keys,
        })
    }

    /// `txids` のうちいちばん後のもの。live の txid は 32 bit なので、`xmax` に近い epoch の値にする。
    fn latest(&self, txids: &[Value]) -> Option<u64> {
        txids
            .iter()
            .filter_map(number)
            .map(|txid| {
                let offset = (txid as u32).wrapping_sub(self.xmax as u32) as i32;
                self.xmax.saturating_add_signed(offset.into())
            })
            .max()
    }

    /// `xid` のトランザクションの変更が、この snapshot に入っているか。
    fn contains(&self, xid: u64) -> bool {
        xid < self.xmin || (xid < self.xmax && !self.in_progress.contains(&xid))
    }
}

pub struct ShapeStream {
    http: reqwest::Client,
    url: String,
    params: Vec<(String, String)>,
    credentials: Credentials,
    handle: Option<String>,
    offset: String,
    cursor: Option<String>,
    live: bool,
    cache_buster: Option<String>,
    rows: HashMap<String, Row>,
    /// `log=changes_only` の購読の開始位置は `now`、通常は `-1`。
    initial_offset: &'static str,
    snapshots: Vec<Snapshot>,
}

impl ShapeStream {
    /// `url` は `{api}/api/shapes/messages` のようなプロキシのパス。`params` は `channel_id` などの追加クエリ。
    pub fn new(
        http: reqwest::Client,
        url: String,
        params: Vec<(String, String)>,
        credentials: Credentials,
    ) -> Self {
        Self {
            http,
            url,
            params,
            credentials,
            handle: None,
            offset: "-1".into(),
            cursor: None,
            live: false,
            cache_buster: None,
            rows: HashMap::new(),
            initial_offset: "-1",
            snapshots: Vec::new(),
        }
    }

    /// 今の時点からの変化だけを受け取る購読（`log=changes_only`）。行は [`Self::subset`] で取る。
    /// 取り直し（409・`must-refetch`）のあとは、同じ行を `subset` で取り直す。
    pub fn changes_only(
        http: reqwest::Client,
        url: String,
        mut params: Vec<(String, String)>,
        credentials: Credentials,
    ) -> Self {
        params.push(("log".into(), "changes_only".into()));
        let mut stream = Self::new(http, url, params, credentials);
        stream.initial_offset = "now";
        stream.offset = "now".into();
        stream
    }

    /// 最初の応答を受け取り、`subset` を取れる状態か。
    pub fn is_started(&self) -> bool {
        self.handle.is_some() && self.offset != self.initial_offset
    }

    /// 条件に合う行だけの snapshot を取り、保持している行に足す。条件の値は `$1` の順に渡す。
    /// 取り直しが要る（409）ときは `None` を返し、行を捨てる。
    pub async fn subset(
        &mut self,
        where_clause: &str,
        values: &[&str],
        order_by: &str,
        limit: usize,
    ) -> Result<Option<Vec<Row>>> {
        let params: HashMap<String, &str> = values
            .iter()
            .enumerate()
            .map(|(index, value)| ((index + 1).to_string(), *value))
            .collect();
        let params = serde_json::to_string(&params)?;
        let limit = limit.to_string();
        let mut query: Vec<(&str, &str)> = self
            .params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        query.push(("offset", &self.offset));
        if let Some(handle) = &self.handle {
            query.push(("handle", handle));
        }
        query.extend([
            ("subset__where", where_clause),
            ("subset__params", params.as_str()),
            ("subset__order_by", order_by),
            ("subset__limit", limit.as_str()),
        ]);
        let request = self.http.get(&self.url).query(&query);
        let response = self.credentials.apply(request).await?.send().await?;
        let headers = response.headers().clone();
        if response.status() == StatusCode::CONFLICT {
            self.restart(header(&headers, "electric-handle"));
            return Ok(None);
        }
        let body = error_for_status(response).await?.text().await?;
        // 別の shape に切り替わった応答は、live の位置と合わないので取り直す。
        if header(&headers, "electric-handle").is_some_and(|handle| Some(handle) != self.handle) {
            self.restart(None);
            return Ok(None);
        }
        let snapshot: Subset =
            serde_json::from_str(&body).context("シェイプの応答を解釈できません")?;
        let mut rows = Vec::new();
        let mut keys = HashSet::new();
        for message in snapshot.data {
            if let Some(Change::Upsert { key, row }) =
                self.apply("insert", message.key, message.value)
            {
                keys.insert(key);
                rows.push(row);
            }
        }
        self.snapshots
            .extend(Snapshot::new(&snapshot.metadata, keys));
        Ok(Some(rows))
    }

    /// 保持している行のうち、`keep` が偽の行を捨てる。一部の行だけを使う購読で、行がたまらないように
    /// する。捨てた行の update は、変わった列だけになる。
    pub fn retain(&mut self, keep: impl Fn(&Row) -> bool) {
        self.rows.retain(|_, row| keep(row));
    }

    /// 保持している行のうち、`id` 列が `id` の行のキー。
    pub fn key_of(&self, id: &str) -> Option<String> {
        self.rows
            .iter()
            .find(|(_, row)| row.get("id").and_then(|value| value.as_str()) == Some(id))
            .map(|(key, _)| key.clone())
    }

    /// 次の変化を1回分取得する。live 中は変化があるかサーバーのタイムアウトまで待つ。
    pub async fn next_batch(&mut self) -> Result<Batch> {
        let response = self.request().await?;
        let status = response.status();
        let headers = response.headers().clone();

        if status == StatusCode::CONFLICT {
            self.restart(header(&headers, "electric-handle"));
            return Ok(Batch {
                reset: true,
                ..Default::default()
            });
        }
        let response = error_for_status(response).await?;
        let messages: Vec<Message> = if status == StatusCode::NO_CONTENT {
            Vec::new()
        } else {
            let body = response.text().await?;
            if body.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&body).context("シェイプの応答を解釈できません")?
            }
        };

        // 本文を読めてから位置を進める。途中で切れたら同じ位置から取り直す。
        self.cache_buster = None;
        if let Some(handle) = header(&headers, "electric-handle") {
            self.handle = Some(handle);
        }
        if let Some(offset) = header(&headers, "electric-offset") {
            self.offset = offset;
        }
        if let Some(cursor) = header(&headers, "electric-cursor") {
            self.cursor = Some(cursor);
        }

        let mut batch = Batch {
            up_to_date: headers.contains_key("electric-up-to-date")
                || status == StatusCode::NO_CONTENT,
            ..Default::default()
        };
        for message in messages {
            match (
                message.headers.control.as_deref(),
                message.headers.operation.as_deref(),
            ) {
                (Some("must-refetch"), _) => {
                    self.restart(None);
                    return Ok(Batch {
                        reset: true,
                        ..Default::default()
                    });
                }
                (Some("up-to-date"), _) => {
                    self.pass(message.headers.global_last_seen_lsn.as_ref());
                    batch.up_to_date = true;
                }
                (Some(_), _) => {}
                (None, Some(operation)) => {
                    self.pass(message.headers.lsn.as_ref());
                    if self.in_snapshot(message.key.as_deref(), message.headers.txids.as_deref()) {
                        continue;
                    }
                    if let Some(change) = self.apply(operation, message.key, message.value) {
                        batch.changes.push(change);
                    }
                }
                (None, None) => {}
            }
        }
        if batch.up_to_date {
            self.live = true;
        }
        Ok(batch)
    }

    async fn request(&self) -> Result<reqwest::Response> {
        let mut query: Vec<(&str, &str)> = self
            .params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        query.push(("offset", &self.offset));
        if let Some(handle) = &self.handle {
            query.push(("handle", handle));
        }
        if self.live {
            query.push(("live", "true"));
            if let Some(cursor) = &self.cursor {
                query.push(("cursor", cursor));
            }
        }
        if let Some(buster) = &self.cache_buster {
            query.push(("cache-buster", buster));
        }
        let request = self.http.get(&self.url).query(&query);
        Ok(self.credentials.apply(request).await?.send().await?)
    }

    /// 行を捨てて最初から取り直す。同じ URL の古い応答を避けるためキャッシュバスターを付ける。
    fn restart(&mut self, handle: Option<String>) {
        self.handle = handle;
        self.offset = self.initial_offset.into();
        self.cursor = None;
        self.live = false;
        self.rows.clear();
        self.snapshots.clear();
        self.cache_buster = Some(uuid::Uuid::new_v4().simple().to_string());
    }

    /// WAL の `lsn` まで届いたら、それより前に取った snapshot の変更はもう届かない。
    fn pass(&mut self, lsn: Option<&Value>) {
        if let Some(lsn) = lsn.and_then(number) {
            self.snapshots
                .retain(|snapshot| snapshot.database_lsn.is_none_or(|taken| taken > lsn));
        }
    }

    /// `key` の `txids` の変更が、取得した snapshot に入っているか。
    fn in_snapshot(&mut self, key: Option<&str>, txids: Option<&[Value]>) -> bool {
        let (Some(key), Some(txids)) = (key, txids) else {
            return false;
        };
        // snapshot より後のトランザクションが届いたら、その snapshot の変更はもう届かない。
        self.snapshots
            .retain(|snapshot| snapshot.latest(txids).is_none_or(|xid| xid < snapshot.xmax));
        self.snapshots.iter().any(|snapshot| {
            snapshot.keys.contains(key)
                && snapshot
                    .latest(txids)
                    .is_some_and(|xid| snapshot.contains(xid))
        })
    }

    fn apply(
        &mut self,
        operation: &str,
        key: Option<String>,
        value: Option<Row>,
    ) -> Option<Change> {
        let key = key?;
        match operation {
            "insert" => {
                let row = value.unwrap_or_default();
                self.rows.insert(key.clone(), row.clone());
                Some(Change::Upsert { key, row })
            }
            "update" => {
                let row = self.rows.entry(key.clone()).or_default();
                row.extend(value.unwrap_or_default());
                Some(Change::Upsert {
                    key,
                    row: row.clone(),
                })
            }
            "delete" => {
                self.rows.remove(&key);
                Some(Change::Delete { key })
            }
            _ => None,
        }
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;

    fn stream(server: &MockServer) -> ShapeStream {
        ShapeStream::new(
            reqwest::Client::new(),
            format!("{}/api/shapes/messages", server.uri()),
            vec![("channel_id".into(), "c1".into())],
            Credentials::Dev("u1".into()),
        )
    }

    fn row(value: Value) -> Row {
        value.as_object().unwrap().clone()
    }

    #[tokio::test]
    async fn syncs_snapshot_then_merges_live_updates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/shapes/messages"))
            .and(query_param("channel_id", "c1"))
            .and(query_param("offset", "-1"))
            .and(query_param_is_missing("live"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("electric-handle", "h1")
                    .insert_header("electric-offset", "0_0")
                    .insert_header("electric-up-to-date", "")
                    .set_body_json(json!([
                        { "headers": { "operation": "insert" }, "key": "k1",
                          "value": { "id": "m1", "content": "hello", "deleted_at": null } },
                        { "headers": { "control": "up-to-date" } }
                    ])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/shapes/messages"))
            .and(query_param("handle", "h1"))
            .and(query_param("offset", "0_0"))
            .and(query_param("live", "true"))
            .and(query_param("cursor", "c9"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("electric-offset", "1_0")
                    .set_body_json(json!([
                        { "headers": { "operation": "update" }, "key": "k1",
                          "value": { "id": "m1", "content": "edited" } },
                        { "headers": { "operation": "delete" }, "key": "k2", "value": { "id": "m2" } }
                    ])),
            )
            .mount(&server)
            .await;

        let mut stream = stream(&server);
        let first = stream.next_batch().await.unwrap();
        assert!(first.up_to_date && !first.reset);
        assert_eq!(first.changes.len(), 1);

        // live 中の cursor は直近の応答のものを使う。
        stream.cursor = Some("c9".into());
        let second = stream.next_batch().await.unwrap();
        assert_eq!(
            second.changes,
            vec![
                Change::Upsert {
                    key: "k1".into(),
                    row: row(json!({ "id": "m1", "content": "edited", "deleted_at": null })),
                },
                Change::Delete { key: "k2".into() },
            ]
        );
        assert_eq!(stream.offset, "1_0");
    }

    #[tokio::test]
    async fn conflict_resets_with_new_handle_and_cache_buster() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("offset", "-1"))
            .and(query_param_is_missing("cache-buster"))
            .respond_with(ResponseTemplate::new(409).insert_header("electric-handle", "h2"))
            .mount(&server)
            .await;

        let mut stream = stream(&server);
        stream.rows.insert("k1".into(), Row::new());
        let batch = stream.next_batch().await.unwrap();
        assert!(batch.reset);
        assert!(stream.rows.is_empty());
        assert_eq!(stream.handle.as_deref(), Some("h2"));
        assert_eq!(stream.offset, "-1");
        assert!(stream.cache_buster.is_some());
        assert!(!stream.live);
    }

    #[tokio::test]
    async fn keeps_position_when_body_is_unreadable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("electric-handle", "h1")
                    .insert_header("electric-offset", "5_0")
                    .insert_header("electric-cursor", "c1")
                    .set_body_string("[{\"headers\":"),
            )
            .mount(&server)
            .await;

        let mut stream = stream(&server);
        assert!(stream.next_batch().await.is_err());
        assert_eq!(stream.offset, "-1");
        assert_eq!(stream.handle, None);
        assert_eq!(stream.cursor, None);
    }

    /// `changes_only` の購読を始め、`m1` を `metadata` の snapshot で取る。
    async fn after_subset(server: &MockServer, metadata: Value, live: Value) -> ShapeStream {
        let headers = |response: ResponseTemplate| {
            response
                .insert_header("electric-handle", "h1")
                .insert_header("electric-offset", "0_0")
                .insert_header("electric-cursor", "1")
        };
        Mock::given(method("GET"))
            .and(query_param("offset", "now"))
            .respond_with(headers(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{ "headers": { "control": "up-to-date" } }])),
            ))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("subset__where", "id = $1"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(json!({
                "metadata": metadata,
                "data": [{ "key": "k1", "value": { "id": "m1", "content": "最新" },
                           "headers": { "operation": "insert" } }],
            }))))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("live", "true"))
            .respond_with(headers(ResponseTemplate::new(200).set_body_json(live)))
            .mount(server)
            .await;
        let mut stream = ShapeStream::changes_only(
            reqwest::Client::new(),
            format!("{}/api/shapes/messages", server.uri()),
            Vec::new(),
            Credentials::Dev("u1".into()),
        );
        stream.next_batch().await.unwrap();
        stream
            .subset("id = $1", &["m1"], "id", 1)
            .await
            .unwrap()
            .unwrap();
        stream
    }

    fn update(key: &str, content: &str, headers: Value) -> Value {
        let mut all = json!({ "operation": "update" });
        all.as_object_mut()
            .unwrap()
            .extend(headers.as_object().unwrap().clone());
        json!({ "key": key, "value": { "content": content }, "headers": all })
    }

    fn contents(batch: &Batch) -> Vec<&str> {
        batch
            .changes
            .iter()
            .filter_map(|change| match change {
                Change::Upsert { row, .. } => row.get("content").and_then(Value::as_str),
                Change::Delete { .. } => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn changes_included_in_a_snapshot_are_not_applied_again() {
        let server = MockServer::start().await;
        let metadata = json!({ "xmin": "10", "xmax": "20", "xip_list": ["15"],
                               "database_lsn": "100", "snapshot_mark": 1 });
        // 12 は snapshot に入っている。15 は取った時点で終わっていない。ほかの行は対象にしない。
        let live = json!([
            update("k1", "古い", json!({ "txids": [12], "lsn": "50" })),
            update("k2", "ほかの行", json!({ "txids": [12], "lsn": "50" })),
            update("k1", "あとで確定", json!({ "txids": [15], "lsn": "60" })),
            update("k1", "後の編集", json!({ "txids": [25], "lsn": "70" })),
        ]);
        let mut stream = after_subset(&server, metadata, live).await;
        let batch = stream.next_batch().await.unwrap();
        assert_eq!(contents(&batch), ["ほかの行", "あとで確定", "後の編集"]);
        assert!(stream.snapshots.is_empty(), "追いついた snapshot は捨てる");
    }

    #[tokio::test]
    async fn snapshots_are_dropped_once_the_log_passes_them() {
        let server = MockServer::start().await;
        // live の txid は 32 bit。epoch をまたいだ値でも、snapshot の前後を判定する。
        let epoch = 1_u64 << 32;
        let metadata = json!({ "xmin": (epoch + 10).to_string(), "xmax": (epoch + 20).to_string(),
                               "xip_list": [], "database_lsn": "100" });
        let live = json!([
            update("k1", "古い", json!({ "txids": [12], "lsn": "50" })),
            update("k1", "前の epoch", json!({ "txids": [u32::MAX], "lsn": "50" })),
            { "headers": { "control": "up-to-date", "global_last_seen_lsn": "100" } },
        ]);
        let mut stream = after_subset(&server, metadata, live).await;
        let batch = stream.next_batch().await.unwrap();
        assert!(contents(&batch).is_empty(), "{batch:?}");
        assert!(stream.snapshots.is_empty());
    }

    #[test]
    fn retains_only_the_rows_in_use() {
        let mut stream = ShapeStream::changes_only(
            reqwest::Client::new(),
            "http://localhost".into(),
            Vec::new(),
            Credentials::Dev("u1".into()),
        );
        for id in ["m1", "m2"] {
            stream.apply("insert", Some(id.into()), Some(row(json!({ "id": id }))));
        }
        stream.retain(|row| row.get("id").and_then(Value::as_str) == Some("m2"));
        assert_eq!(stream.key_of("m1"), None);
        assert_eq!(stream.key_of("m2").as_deref(), Some("m2"));
    }

    #[tokio::test]
    async fn must_refetch_control_resets() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("electric-handle", "h1")
                    .set_body_json(json!([{ "headers": { "control": "must-refetch" } }])),
            )
            .mount(&server)
            .await;

        let mut stream = stream(&server);
        stream.live = true;
        let batch = stream.next_batch().await.unwrap();
        assert_eq!(
            batch,
            Batch {
                reset: true,
                ..Default::default()
            }
        );
        assert!(!stream.live);
        assert_eq!(stream.handle, None);
    }
}
