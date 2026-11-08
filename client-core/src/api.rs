//! backend の REST API。読み取りは Electric のシェイプで行うため、ここは書き込みと
//! シェイプにない情報 (自分、既読、入力中) だけを扱う。

use anyhow::Result;
use reqwest::Method;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::http::{Credentials, error_for_status};

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    credentials: Credentials,
}

/// `GET /api/users/me`
#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
}

/// `GET /api/channel-read-states` の1件。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReadState {
    pub channel_id: String,
    pub has_unread: bool,
}

/// `GET /api/channels/{id}/typing`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TypingUser {
    pub id: String,
    pub display_name: String,
}

/// 書き込み API の `{data, txid}`。`txid` が null のときは何も変わっていない。
#[derive(Debug, Deserialize)]
struct Envelope {
    txid: Option<String>,
}

#[derive(Serialize)]
struct NewMessage<'a> {
    id: &'a str,
    channel_id: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
}

impl ApiClient {
    pub fn new(http: reqwest::Client, base_url: &str, credentials: Credentials) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            credentials,
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header(reqwest::header::ACCEPT, "application/json");
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

    pub async fn me(&self) -> Result<Me> {
        self.get("/api/users/me").await
    }

    /// `id` はクライアントで採番する (frontend と同じく、送信直後の行と突き合わせるため)。
    pub async fn send_message(
        &self,
        id: &str,
        channel_id: &str,
        content: &str,
        parent_id: Option<&str>,
    ) -> Result<Option<String>> {
        let body = serde_json::to_value(NewMessage {
            id,
            channel_id,
            content,
            parent_id,
        })?;
        self.write(Method::POST, "/api/messages", Some(body)).await
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
        let path = format!("/api/channels/{}/typing", segment(channel_id));
        let body = json!({ "parent_id": parent_id, "session_id": session_id, "active": active });
        self.send(Method::POST, &path, Some(body)).await?;
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
}
