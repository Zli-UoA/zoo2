//! LiveKit の接続トークン（JWT）と、Twirp API（RoomService）の呼び出し。
//! https://docs.livekit.io/home/get-started/authentication/

use serde::Serialize;
use serde_json::json;

use super::config::LiveKitConfig;
use crate::error::AppError;

/// 参加者のトークンの有効期限。`sfu/join` の応答ですぐに接続するので短くする。
///
/// セルフホストの LiveKit では、`RemoveParticipant` が発行済みのトークンを無効にしない。退出・キックの後に
/// 手元のトークンで入り直せる時間を、この長さまでに抑える。接続後の再接続は、クライアント SDK が LiveKit
/// サーバーから受け取る更新トークンで行うので、この期限には縛られない（残る時間窓は docs/calls-sfu.md）。
pub const JOIN_TOKEN_TTL_SECS: u64 = 60;
/// sfu-bridge のトークンの有効期限。bridge は退出させる対象ではなく、rooms の long-poll（最大 25 秒）の
/// たびに新しいトークンを受け取る。
pub const BRIDGE_TOKEN_TTL_SECS: u64 = 5 * 60;
const ADMIN_TOKEN_TTL_SECS: u64 = 60;

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VideoGrant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub room_join: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub room_admin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_publish: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_subscribe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_publish_data: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_publish_sources: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_update_own_metadata: Option<bool>,
}

#[derive(Debug, Serialize)]
struct Claims<'a> {
    iss: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    nbf: u64,
    exp: u64,
    video: VideoGrant,
}

/// 通話ごとの LiveKit のルーム名。
pub fn room_name(channel_id: &str, epoch: i64) -> String {
    format!("call-{channel_id}-{epoch}")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct LiveKit {
    http: reqwest::Client,
    config: LiveKitConfig,
}

impl LiveKit {
    pub fn new(http: reqwest::Client, config: LiveKitConfig) -> Self {
        Self { http, config }
    }

    pub fn ws_url(&self) -> &str {
        &self.config.ws_url
    }

    pub fn config(&self) -> &LiveKitConfig {
        &self.config
    }

    fn sign(
        &self,
        identity: Option<&str>,
        name: Option<&str>,
        ttl: u64,
        video: VideoGrant,
    ) -> Result<String, AppError> {
        let now = now_secs();
        let claims = Claims {
            iss: &self.config.api_key,
            sub: identity,
            name,
            // 時計のずれを少し許す。
            nbf: now.saturating_sub(10),
            exp: now + ttl,
            video,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(self.config.api_secret.as_bytes()),
        )
        .map_err(|e| AppError::Internal(anyhow::anyhow!("LiveKit token: {e}")))
    }

    /// 人間の参加者のトークン。マイクの publish と購読だけを許す。identity は call の session_id。
    pub fn participant_token(
        &self,
        room: &str,
        session_id: &str,
        display_name: &str,
    ) -> Result<String, AppError> {
        self.sign(
            Some(session_id),
            Some(display_name),
            JOIN_TOKEN_TTL_SECS,
            VideoGrant {
                room: Some(room.to_string()),
                room_join: true,
                can_publish: Some(true),
                can_subscribe: Some(true),
                can_publish_data: Some(false),
                can_publish_sources: Some(vec!["microphone"]),
                can_update_own_metadata: Some(false),
                ..Default::default()
            },
        )
    }

    /// sfu-bridge のトークン。Cloudflare 側の人の声を publish するので、マイク以外の source も許さない。
    pub fn bridge_token(&self, room: &str, bridge_epoch: i64) -> Result<String, AppError> {
        let identity = format!("bridge:{bridge_epoch}");
        self.sign(
            Some(&identity),
            Some("bridge"),
            BRIDGE_TOKEN_TTL_SECS,
            VideoGrant {
                room: Some(room.to_string()),
                room_join: true,
                can_publish: Some(true),
                can_subscribe: Some(true),
                can_publish_data: Some(false),
                can_publish_sources: Some(vec!["microphone"]),
                can_update_own_metadata: Some(false),
                ..Default::default()
            },
        )
    }

    /// 退出した参加者をルームから外す。もういなければ成功として扱う。
    pub async fn remove_participant(&self, room: &str, identity: &str) -> Result<(), AppError> {
        let token = self.sign(
            None,
            None,
            ADMIN_TOKEN_TTL_SECS,
            VideoGrant {
                room: Some(room.to_string()),
                room_admin: true,
                ..Default::default()
            },
        )?;
        let res = self
            .http
            .post(format!(
                "{}/twirp/livekit.RoomService/RemoveParticipant",
                self.config.api_url
            ))
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(10))
            .json(&json!({ "room": room, "identity": identity }))
            .send()
            .await
            .map_err(|e| AppError::BadGateway(format!("LiveKit: {e}")))?;
        let status = res.status();
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        let body = res.text().await.unwrap_or_default();
        // Twirp の not_found は 404。ルームがもうないときもここに来る。
        if body.contains("\"not_found\"") {
            return Ok(());
        }
        Err(AppError::BadGateway(format!("LiveKit returned {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::Cidr;
    use super::*;

    fn lk() -> LiveKit {
        LiveKit::new(
            reqwest::Client::new(),
            LiveKitConfig {
                ws_url: "wss://lk".into(),
                api_url: "http://127.0.0.1:7880".into(),
                api_key: "key".into(),
                api_secret: "0123456789abcdef0123456789abcdef".into(),
                lan_egress: vec![Cidr::parse("10.0.0.0/8").unwrap()],
            },
        )
    }

    fn decode(token: &str) -> serde_json::Value {
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        v.validate_aud = false;
        jsonwebtoken::decode::<serde_json::Value>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(b"0123456789abcdef0123456789abcdef"),
            &v,
        )
        .unwrap()
        .claims
    }

    #[test]
    fn participant_token_is_limited_to_microphone_in_one_room() {
        let claims = decode(
            &lk()
                .participant_token("call-c1-3", "sess", "Alice")
                .unwrap(),
        );
        assert_eq!(claims["iss"], "key");
        assert_eq!(claims["sub"], "sess");
        assert_eq!(claims["name"], "Alice");
        let video = &claims["video"];
        assert_eq!(video["room"], "call-c1-3");
        assert_eq!(video["roomJoin"], true);
        assert_eq!(video["canPublishSources"], json!(["microphone"]));
        assert_eq!(video["canPublishData"], false);
        assert!(video.get("roomAdmin").is_none());
        assert!(video.get("roomCreate").is_none());
        let ttl = claims["exp"].as_u64().unwrap() - claims["nbf"].as_u64().unwrap();
        assert!(ttl <= JOIN_TOKEN_TTL_SECS + 10);
    }

    #[test]
    fn bridge_identity_carries_epoch() {
        let claims = decode(&lk().bridge_token("call-c1-3", 7).unwrap());
        assert_eq!(claims["sub"], "bridge:7");
        assert!(claims["video"].get("hidden").is_none());
    }

    #[test]
    fn room_name_changes_with_epoch() {
        assert_ne!(room_name("c1", 1), room_name("c1", 2));
    }
}
