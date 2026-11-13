//! backend の REST API。読み取りは Electric のシェイプで行うため、ここは書き込みと
//! シェイプにない情報 (自分、既読、入力中、スレッド一覧) だけを扱う。

use anyhow::Result;
use reqwest::Method;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::http::{Credentials, TokenPersistenceReport, error_for_status};

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    credentials: Credentials,
    typing_write: std::sync::Arc<tokio::sync::Mutex<()>>,
}

pub use zoo2_client_types::api::Me;

/// `GET /api/channel-read-states` の1件。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReadState {
    pub channel_id: String,
    pub has_unread: bool,
}

/// 閲覧権限で絞り込まれた `GET /api/threads` の1件。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub channel_id: String,
    pub content: String,
    pub thread_archive_at: Option<String>,
    pub reply_count: u64,
}

/// スレッド一覧の1ページ。次ページは `next_cursor` をそのまま渡す。
#[derive(Debug, Clone, Deserialize)]
pub struct ThreadPage {
    pub items: Vec<ThreadSummary>,
    pub next_cursor: Option<String>,
}

/// `GET /api/channels/{id}/typing`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TypingUser {
    pub id: String,
    pub display_name: String,
}

/// `POST /api/attachments/presign`
#[derive(Debug, Clone, Deserialize)]
pub struct PresignAttachmentResponse {
    pub attachment_id: String,
    pub upload_url: String,
    pub storage_key: String,
}

/// `GET /api/attachments/{id}/url`
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentUrlResponse {
    pub id: String,
    pub download_url: String,
    pub file_name: String,
    pub file_size: i64,
    pub content_type: String,
}

/// `POST /api/channels` で作成されたチャンネル
#[derive(Debug, Clone, Deserialize)]
pub struct CreatedChannel {
    pub id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub is_private: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MyProfile {
    /// timestamptz の JSON 文字列。`notify::parse_dnd_until` で読む。
    #[serde(default)]
    pub dnd_until: Option<String>,
}

/// 書き込み API の `{data, txid}`。`txid` が null のときは何も変わっていない。
#[derive(Debug, Deserialize)]
struct Envelope<T = Value> {
    data: Option<T>,
    txid: Option<String>,
}

#[derive(Serialize)]
struct NewMessage<'a> {
    id: &'a str,
    channel_id: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attachment_ids: Option<Vec<String>>,
}

impl ApiClient {
    pub fn new(http: reqwest::Client, base_url: &str, credentials: Credentials) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            credentials,
            typing_write: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let reading = method == Method::GET;
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header(reqwest::header::ACCEPT, "application/json");
        if reading {
            request = request.timeout(std::time::Duration::from_secs(10));
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let request = self.credentials.apply(request).await?;
        error_for_status(request.send().await?).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self.send(Method::GET, path, None).await?.json().await?)
    }

    /// 書き込みを送り、シェイプで反映を待つための txid を返す。
    async fn write(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Option<String>> {
        let envelope: Envelope = self.send(method, path, body).await?.json().await?;
        Ok(envelope.txid)
    }

    pub async fn account_profiles(&self) -> Result<zoo2_client_types::accounts::AccountProfiles> {
        let value: Value = self.get("/api/emoji-accounts").await?;
        let profiles: Vec<zoo2_client_types::accounts::AccountProfile> =
            serde_json::from_value(value["accounts"].clone())?;
        Ok(profiles
            .into_iter()
            .map(|p| (p.username.clone(), p))
            .collect())
    }

    pub async fn emoji_catalog(
        &self,
    ) -> Result<(
        std::collections::HashMap<String, String>,
        std::collections::HashSet<String>,
    )> {
        let (standard, custom) = tokio::join!(
            self.get::<Value>("/api/emoji-shortcodes"),
            self.get::<Vec<Value>>("/api/emojis")
        );
        // Custom names must still win when the standard service is unavailable.
        let shortcodes = standard
            .ok()
            .and_then(|value| serde_json::from_value(value["shortcodes"].clone()).ok())
            .unwrap_or_default();
        let mut names = std::collections::HashSet::new();
        for row in custom? {
            if let Some(name) = row["name"].as_str() {
                names.insert(name.to_owned());
            }
            if let Some(aliases) = row["aliases"].as_array() {
                for alias in aliases {
                    if let Some(name) = alias.as_str() {
                        names.insert(name.to_owned());
                    }
                }
            }
        }
        Ok((shortcodes, names))
    }
    pub async fn me(&self) -> Result<Me> {
        self.get("/api/users/me").await
    }

    pub async fn finish_token_tasks(&self) -> TokenPersistenceReport {
        self.credentials.finish_token_tasks().await
    }

    pub async fn token_persistence_report(&self) -> TokenPersistenceReport {
        self.credentials.persistence_report().await
    }

    pub async fn threads(&self, archived: bool, before: Option<&str>) -> Result<ThreadPage> {
        self.thread_page(archived, before, None).await
    }

    pub async fn thread_page(
        &self,
        archived: bool,
        before: Option<&str>,
        through: Option<&str>,
    ) -> Result<ThreadPage> {
        let mut path = format!("/api/threads?archived={archived}");
        if let Some(cursor) = before {
            path.push_str(&format!("&before={}", segment(cursor)));
        }
        if let Some(cursor) = through {
            path.push_str(&format!("&through={}", segment(cursor)));
        }
        self.get(&path).await
    }

    pub async fn unarchive_thread(&self, id: &str) -> Result<Option<String>> {
        let path = format!("/api/threads/{}/unarchive", segment(id));
        self.write(Method::POST, &path, None).await
    }

    /// `GET /api/users/me/profile` のうち、クライアントが使う部分。
    /// `dnd_until` は `preferences:read` の scope がないと返らない (None になる)。
    pub async fn my_profile(&self) -> Result<MyProfile> {
        self.get("/api/users/me/profile").await
    }

    /// `id` はクライアントで採番する (frontend と同じく、送信直後の行と突き合わせるため)。
    pub async fn send_message(
        &self,
        id: &str,
        channel_id: &str,
        content: &str,
        parent_id: Option<&str>,
    ) -> Result<Option<String>> {
        self.send_message_with_attachments(id, channel_id, content, parent_id, None)
            .await
    }

    /// 添付ファイル ID を含むメッセージ送信。
    pub async fn send_message_with_attachments(
        &self,
        id: &str,
        channel_id: &str,
        content: &str,
        parent_id: Option<&str>,
        attachment_ids: Option<Vec<String>>,
    ) -> Result<Option<String>> {
        let body = serde_json::to_value(NewMessage {
            id,
            channel_id,
            content,
            parent_id,
            attachment_ids,
        })?;
        self.write(Method::POST, "/api/messages", Some(body)).await
    }

    /// 添付ファイルの署名付きアップロード URL を発行する。
    pub async fn presign_attachment(
        &self,
        channel_id: &str,
        file_name: &str,
        file_size: i64,
        content_type: &str,
    ) -> Result<PresignAttachmentResponse> {
        let body = json!({
            "channel_id": channel_id,
            "file_name": file_name,
            "file_size": file_size,
            "content_type": content_type,
        });
        let envelope: Envelope<PresignAttachmentResponse> = self
            .send(Method::POST, "/api/attachments/presign", Some(body))
            .await?
            .json()
            .await?;
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("Presign レスポンスにデータがありません"))
    }

    /// 署名付き URL へバイナリデータをアップロードする。
    pub async fn upload_attachment_bytes(
        &self,
        upload_url: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let response = self
            .http
            .put(upload_url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await?;
        error_for_status(response).await?;
        Ok(())
    }

    /// 添付ファイルの署名付きダウンロード URL を取得する。
    pub async fn get_attachment_url(&self, attachment_id: &str) -> Result<AttachmentUrlResponse> {
        let path = format!("/api/attachments/{}/url", segment(attachment_id));
        let envelope: Envelope<AttachmentUrlResponse> = self.get(&path).await?;
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("Attachment URL レスポンスにデータがありません"))
    }

    /// チャンネルを作成する。
    pub async fn create_channel(
        &self,
        id: Option<&str>,
        channel_name: &str,
        description: Option<&str>,
        is_private: bool,
    ) -> Result<(CreatedChannel, Option<String>)> {
        let mut body = json!({
            "channel_name": channel_name,
            "description": description,
            "is_private": is_private,
        });
        if let Some(id_val) = id {
            body["id"] = json!(id_val);
        }
        let envelope: Envelope<CreatedChannel> = self
            .send(Method::POST, "/api/channels", Some(body))
            .await?
            .json()
            .await?;
        let channel = envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("作成されたチャンネルのデータがありません"))?;
        Ok((channel, envelope.txid))
    }

    pub async fn edit_message(&self, id: &str, content: &str) -> Result<Option<String>> {
        let path = format!("/api/messages/{}", segment(id));
        self.write(Method::PATCH, &path, Some(json!({ "content": content })))
            .await
    }

    pub async fn delete_message(&self, id: &str) -> Result<Option<String>> {
        let path = format!("/api/messages/{}", segment(id));
        self.write(Method::DELETE, &path, None).await
    }

    pub async fn add_reaction(
        &self,
        reaction_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<Option<String>> {
        let path = format!("/api/messages/{}/reactions", segment(message_id));
        let body = json!({ "id": reaction_id, "emoji": emoji });
        self.write(Method::POST, &path, Some(body)).await
    }

    pub async fn remove_reaction(&self, message_id: &str, emoji: &str) -> Result<Option<String>> {
        let path = format!(
            "/api/messages/{}/reactions?emoji={}",
            segment(message_id),
            segment(emoji)
        );
        self.write(Method::DELETE, &path, None).await
    }

    pub async fn join_channel(&self, channel_id: &str) -> Result<Option<String>> {
        let path = format!("/api/channels/{}/join", segment(channel_id));
        self.write(Method::POST, &path, None).await
    }

    pub async fn leave_channel(&self, channel_id: &str) -> Result<Option<String>> {
        let path = format!("/api/channels/{}/leave", segment(channel_id));
        self.write(Method::POST, &path, None).await
    }

    pub async fn read_states(&self) -> Result<Vec<ReadState>> {
        self.get("/api/channel-read-states").await
    }

    /// `message_id` はチャンネルの最新のルートメッセージ。既読位置は前にしか進まない。
    pub async fn mark_read(&self, channel_id: &str, message_id: &str) -> Result<Option<String>> {
        let path = format!("/api/channels/{}/read-state", segment(channel_id));
        self.write(
            Method::POST,
            &path,
            Some(json!({ "message_id": message_id })),
        )
        .await
    }

    /// 入力中の状態を送る。backend は6秒で失効させるので、入力中は数秒おきに送り直す。
    pub async fn set_typing(
        &self,
        channel_id: &str,
        parent_id: Option<&str>,
        session_id: &str,
        active: bool,
    ) -> Result<()> {
        // Repeated renewals may be skipped while a previous write is pending. Stops must
        // wait for it, so a slow start cannot arrive after the lease was stopped.
        let _guard = if active {
            match self.typing_write.try_lock() {
                Ok(guard) => guard,
                Err(_) => return Ok(()),
            }
        } else {
            self.typing_write.lock().await
        };
        let path = format!("/api/channels/{}/typing", segment(channel_id));
        let body = json!({ "parent_id": parent_id, "session_id": session_id, "active": active });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.send(Method::POST, &path, Some(body)),
        )
        .await??;
        Ok(())
    }

    /// 自分以外の入力中ユーザー。
    pub async fn typing(
        &self,
        channel_id: &str,
        parent_id: Option<&str>,
    ) -> Result<Vec<TypingUser>> {
        let mut path = format!("/api/channels/{}/typing", segment(channel_id));
        if let Some(parent_id) = parent_id {
            path.push_str(&format!("?parent_id={}", segment(parent_id)));
        }
        self.get(&path).await
    }
}

/// パスとクエリに埋め込む値をエスケープする。
fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path, query_param},
    };

    use super::*;
    use crate::http::HttpError;

    fn client(server: &MockServer) -> ApiClient {
        ApiClient::new(
            reqwest::Client::new(),
            &format!("{}/", server.uri()),
            Credentials::Dev("u1".into()),
        )
    }

    #[tokio::test]
    async fn slow_typing_renewals_do_not_overlap_and_stop_follows_the_start() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channels/c1/typing"))
            .respond_with(
                ResponseTemplate::new(204).set_delay(std::time::Duration::from_millis(30)),
            )
            .expect(2)
            .mount(&server)
            .await;
        let api = client(&server);
        let first_api = api.clone();
        let first =
            tokio::spawn(async move { first_api.set_typing("c1", None, "session", true).await });
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        api.set_typing("c1", None, "session", true).await.unwrap();
        api.set_typing("c1", None, "session", false).await.unwrap();
        first.await.unwrap().unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].body_json::<Value>().unwrap()["active"], true);
        assert_eq!(requests[1].body_json::<Value>().unwrap()["active"], false);
    }

    #[tokio::test]
    async fn custom_names_survive_standard_dictionary_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/emoji-shortcodes"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/emojis"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name":"smile", "aliases":["custom_alias"]}
            ])))
            .mount(&server)
            .await;
        let (standard, custom) = client(&server).emoji_catalog().await.unwrap();
        assert!(standard.is_empty());
        assert!(custom.contains("smile"));
        assert!(custom.contains("custom_alias"));
    }

    #[tokio::test]
    async fn send_message_posts_body_with_dev_user_and_returns_txid() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/messages"))
            .and(header("X-Dev-User", "u1"))
            .and(body_json(json!({
                "id": "m1", "channel_id": "c1", "content": "hi", "parent_id": "p1"
            })))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({ "data": { "id": "m1" }, "txid": "42" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let txid = client(&server)
            .send_message("m1", "c1", "hi", Some("p1"))
            .await
            .unwrap();
        assert_eq!(txid.as_deref(), Some("42"));
    }

    #[tokio::test]
    async fn remove_reaction_escapes_emoji_in_query() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/messages/m1/reactions"))
            .and(query_param("emoji", "👍"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let txid = client(&server).remove_reaction("m1", "👍").await.unwrap();
        assert_eq!(txid, None);
    }

    #[tokio::test]
    async fn lists_threads_and_reopens_with_the_authenticated_api() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/threads")).and(wiremock::matchers::query_param("archived", "false"))
            .and(header("X-Dev-User", "u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[
                {"id":"reply1","channel_id":"c1","content":"Nested thread","reply_count":2,"thread_archive_at":null}
            ],"next_cursor":null}))).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/threads/reply1/unarchive"))
            .and(header("X-Dev-User", "u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":{"id":"reply1","thread_archive_at":null},"txid":"43"}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/threads"))
            .and(wiremock::matchers::query_param("archived", "true"))
            .and(wiremock::matchers::query_param("before", "cursor /"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"items":[],"next_cursor":null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let api = client(&server);
        assert!(
            api.threads(true, Some("cursor /"))
                .await
                .unwrap()
                .items
                .is_empty()
        );
        let threads = api.threads(false, None).await.unwrap();
        assert_eq!(threads.items[0].reply_count, 2);
        assert!(threads.items[0].thread_archive_at.is_none());
        assert_eq!(
            api.unarchive_thread(&threads.items[0].id)
                .await
                .unwrap()
                .as_deref(),
            Some("43")
        );
    }

    #[tokio::test]
    async fn reads_me_and_read_states() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "u1", "user_name": "alice", "display_name": "Alice",
                "is_bot": false, "created_at": "2026-11-01T00:00:00", "is_admin": false
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/channel-read-states"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "channel_id": "c1", "last_read_at": null, "has_unread": true }
            ])))
            .mount(&server)
            .await;

        let api = client(&server);
        assert_eq!(api.me().await.unwrap().user_name, "alice");
        assert_eq!(
            api.read_states().await.unwrap(),
            vec![ReadState {
                channel_id: "c1".into(),
                has_unread: true
            }]
        );
    }

    #[tokio::test]
    async fn maps_error_body_and_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channels/c1/join"))
            .respond_with(
                ResponseTemplate::new(404).set_body_json(json!({ "error": "Channel not found" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/channels/c1/leave"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let api = client(&server);
        let err = api.join_channel("c1").await.unwrap_err();
        match err.downcast_ref::<HttpError>() {
            Some(HttpError::Status { status, message }) => {
                assert_eq!(status.as_u16(), 404);
                assert_eq!(message, "Channel not found");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        let err = api.leave_channel("c1").await.unwrap_err();
        assert!(matches!(
            err.downcast_ref::<HttpError>(),
            Some(HttpError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn typing_sends_scope_and_lists_others() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channels/c1/typing"))
            .and(body_json(
                json!({ "parent_id": null, "session_id": "s1", "active": true }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/channels/c1/typing"))
            .and(query_param("parent_id", "p1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{ "id": "u2", "display_name": "Bob" }])),
            )
            .mount(&server)
            .await;

        let api = client(&server);
        api.set_typing("c1", None, "s1", true).await.unwrap();
        let users = api.typing("c1", Some("p1")).await.unwrap();
        assert_eq!(users[0].display_name, "Bob");
    }

    #[tokio::test]
    async fn send_message_with_attachments_and_presign_and_create_channel() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/messages"))
            .and(body_json(json!({
                "id": "m1", "channel_id": "c1", "content": "with attachment",
                "attachment_ids": ["att1"]
            })))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({ "data": { "id": "m1" }, "txid": "99" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/attachments/presign"))
            .and(body_json(json!({
                "channel_id": "c1", "file_name": "pic.png", "file_size": 1024, "content_type": "image/png"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "attachment_id": "att1",
                    "upload_url": "https://s3.example.com/upload",
                    "storage_key": "keys/pic.png"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/attachments/att1/url"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "id": "att1",
                    "download_url": "https://s3.example.com/download",
                    "file_name": "pic.png",
                    "file_size": 1024,
                    "content_type": "image/png"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/channels"))
            .and(body_json(json!({
                "channel_name": "dev", "description": "Dev channel", "is_private": false
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "data": {
                    "id": "c_new",
                    "channel_name": "dev",
                    "description": "Dev channel",
                    "is_private": false
                },
                "txid": "100"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let api = client(&server);

        let txid = api
            .send_message_with_attachments(
                "m1",
                "c1",
                "with attachment",
                None,
                Some(vec!["att1".into()]),
            )
            .await
            .unwrap();
        assert_eq!(txid.as_deref(), Some("99"));

        let presign = api
            .presign_attachment("c1", "pic.png", 1024, "image/png")
            .await
            .unwrap();
        assert_eq!(presign.attachment_id, "att1");
        assert_eq!(presign.upload_url, "https://s3.example.com/upload");

        let url_info = api.get_attachment_url("att1").await.unwrap();
        assert_eq!(url_info.download_url, "https://s3.example.com/download");

        let (created, txid) = api
            .create_channel(None, "dev", Some("Dev channel"), false)
            .await
            .unwrap();
        assert_eq!(created.id, "c_new");
        assert_eq!(created.channel_name, "dev");
        assert_eq!(txid.as_deref(), Some("100"));
    }
}
