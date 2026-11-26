//! Electric v1 の HTTP シェイププロトコル。
//!
//! 1. `offset=-1` で初回取得し、`electric-handle` と `electric-offset` を追う。
//! 2. `up-to-date` を受けたら `live=true` と `electric-cursor` を付けたロングポーリングに移る。
//! 3. 409 か `must-refetch` を受けたら行を捨て、新しい handle で最初から取り直す。
//!
//! update は既定 (`replica=default`) で変更列と主キーだけが届くため、ここで行を保持して
//! マージし、呼び出し側には常に完全な行を渡す。

use std::collections::HashMap;

use anyhow::{Context, Result};
use reqwest::{StatusCode, header::HeaderMap};
use serde::Deserialize;

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
    data: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct MessageHeaders {
    operation: Option<String>,
    control: Option<String>,
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
        for message in snapshot.data {
            if let Some(Change::Upsert { row, .. }) =
                self.apply("insert", message.key, message.value)
            {
                rows.push(row);
            }
        }
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
                (Some("up-to-date"), _) => batch.up_to_date = true,
                (Some(_), _) => {}
                (None, Some(operation)) => {
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
        self.cache_buster = Some(uuid::Uuid::new_v4().simple().to_string());
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
