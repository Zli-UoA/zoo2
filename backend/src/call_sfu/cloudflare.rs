//! Cloudflare Realtime（SFU API）の呼び出し。app secret は backend だけが持ち、クライアントには渡さない。
//! API: https://developers.cloudflare.com/realtime/sfu/https-api/

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::config::CloudflareConfig;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDescription {
    #[serde(rename = "type")]
    pub kind: String,
    pub sdp: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracksResponse {
    #[serde(default)]
    pub session_description: Option<SessionDescription>,
    #[serde(default)]
    pub requires_immediate_renegotiation: bool,
    #[serde(default)]
    pub tracks: Vec<TrackResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackResult {
    #[serde(default)]
    pub mid: Option<String>,
    #[serde(default)]
    pub track_name: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionState {
    #[serde(default)]
    tracks: Vec<SessionTrack>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionTrack {
    #[serde(default)]
    mid: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// pull する相手の track。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTrack {
    pub session_id: String,
    pub track_name: String,
}

#[derive(Clone)]
pub struct CloudflareRealtime {
    http: reqwest::Client,
    config: CloudflareConfig,
}

impl CloudflareRealtime {
    pub fn new(http: reqwest::Client, config: CloudflareConfig) -> Self {
        Self { http, config }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/apps/{}/{}",
            self.config.api_base, self.config.app_id, path
        )
    }

    async fn send<T: for<'de> Deserialize<'de>>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T, AppError> {
        let res = req
            .bearer_auth(&self.config.app_secret)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::BadGateway(format!("Cloudflare Realtime: {e}")))?;
        let status = res.status();
        let body = res
            .text()
            .await
            .map_err(|e| AppError::BadGateway(format!("Cloudflare Realtime: {e}")))?;
        if !status.is_success() {
            // 応答の本文に秘密は入らないが、長さは抑える。
            let snippet: String = body.chars().take(300).collect();
            tracing::warn!(%status, body = %snippet, "Cloudflare Realtime request failed");
            return Err(AppError::BadGateway(format!(
                "Cloudflare Realtime returned {status}"
            )));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| AppError::BadGateway(format!("Cloudflare Realtime: {e}")))?;
        if let Some(code) = value.get("errorCode").and_then(|v| v.as_str()) {
            let desc = value
                .get("errorDescription")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            tracing::warn!(code, desc, "Cloudflare Realtime returned an error");
            return Err(AppError::BadGateway(format!(
                "Cloudflare Realtime error: {code}"
            )));
        }
        serde_json::from_value(value)
            .map_err(|e| AppError::BadGateway(format!("Cloudflare Realtime: {e}")))
    }

    pub async fn new_session(&self) -> Result<String, AppError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct NewSession {
            session_id: String,
        }
        let res: NewSession = self
            .send(self.http.post(self.url("sessions/new")).json(&json!({})))
            .await?;
        Ok(res.session_id)
    }

    /// 自分のマイクの track を push する。`offer` はクライアントの SDP、`mid` はその音声の m-line。
    pub async fn push_track(
        &self,
        session_id: &str,
        offer: &SessionDescription,
        mid: &str,
        track_name: &str,
    ) -> Result<TracksResponse, AppError> {
        let body = json!({
            "sessionDescription": offer,
            "tracks": [{ "location": "local", "mid": mid, "trackName": track_name }],
        });
        self.send(
            self.http
                .post(self.url(&format!("sessions/{session_id}/tracks/new")))
                .json(&body),
        )
        .await
    }

    /// ほかの人の track を pull する。応答に offer が入っていれば、クライアントが answer を返す。
    pub async fn pull_tracks(
        &self,
        session_id: &str,
        remotes: &[RemoteTrack],
    ) -> Result<TracksResponse, AppError> {
        let tracks: Vec<_> = remotes
            .iter()
            .map(|r| {
                json!({ "location": "remote", "sessionId": r.session_id, "trackName": r.track_name })
            })
            .collect();
        self.send(
            self.http
                .post(self.url(&format!("sessions/{session_id}/tracks/new")))
                .json(&json!({ "tracks": tracks })),
        )
        .await
    }

    pub async fn renegotiate(
        &self,
        session_id: &str,
        answer: &SessionDescription,
    ) -> Result<(), AppError> {
        let _: serde_json::Value = self
            .send(
                self.http
                    .put(self.url(&format!("sessions/{session_id}/renegotiate")))
                    .json(&json!({ "sessionDescription": answer })),
            )
            .await?;
        Ok(())
    }

    /// 指定した m-line の track を閉じる。
    pub async fn close_tracks(&self, session_id: &str, mids: &[String]) -> Result<(), AppError> {
        if mids.is_empty() {
            return Ok(());
        }
        let tracks: Vec<_> = mids.iter().map(|m| json!({ "mid": m })).collect();
        let _: serde_json::Value = self
            .send(
                self.http
                    .put(self.url(&format!("sessions/{session_id}/tracks/close")))
                    .json(&json!({ "tracks": tracks, "force": true })),
            )
            .await?;
        Ok(())
    }

    /// セッションの track をすべて強制的に閉じる（退出・キックの後始末）。
    pub async fn close_session(&self, session_id: &str) -> Result<(), AppError> {
        let state: SessionState = self
            .send(self.http.get(self.url(&format!("sessions/{session_id}"))))
            .await?;
        let mids: Vec<String> = state
            .tracks
            .into_iter()
            .filter(|t| t.status.as_deref() != Some("inactive"))
            .filter_map(|t| t.mid)
            .collect();
        self.close_tracks(session_id, &mids).await
    }
}

/// push する SDP の検査。音声の m-line がちょうど 1 本で、ほかのメディアがないこと。
/// 戻り値はその m-line の mid。
pub fn validate_audio_only_offer(desc: &SessionDescription) -> Result<String, AppError> {
    if desc.kind != "offer" {
        return Err(AppError::BadRequest(
            "sessionDescription must be an offer".into(),
        ));
    }
    if desc.sdp.len() > 16 * 1024 {
        return Err(AppError::BadRequest("SDP is too large".into()));
    }
    let mut media = Vec::new();
    for line in desc.sdp.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("m=") {
            media.push((
                rest.split(' ').next().unwrap_or("").to_string(),
                None::<String>,
            ));
        } else if let Some(mid) = line.strip_prefix("a=mid:")
            && let Some(last) = media.last_mut()
        {
            last.1 = Some(mid.trim().to_string());
        }
    }
    match media.as_slice() {
        [(kind, Some(mid))] if kind == "audio" => Ok(mid.clone()),
        _ => Err(AppError::BadRequest(
            "Offer must contain exactly one audio m-line with a mid".into(),
        )),
    }
}

/// bridge の SDP の検査。音声以外の m-line がなく、`mid` の m-line があること。
pub fn validate_audio_offer_mid(desc: &SessionDescription, mid: &str) -> Result<(), AppError> {
    if desc.kind != "offer" {
        return Err(AppError::BadRequest(
            "sessionDescription must be an offer".into(),
        ));
    }
    if desc.sdp.len() > 64 * 1024 {
        return Err(AppError::BadRequest("SDP is too large".into()));
    }
    let mut found = false;
    let mut current_audio = false;
    for line in desc.sdp.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("m=") {
            current_audio = rest.starts_with("audio ");
            if !current_audio {
                return Err(AppError::BadRequest("Offer must contain audio only".into()));
            }
        } else if let Some(m) = line.strip_prefix("a=mid:") {
            found |= current_audio && m.trim() == mid;
        }
    }
    if found {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "Offer has no audio m-line with that mid".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(sdp: &str) -> SessionDescription {
        SessionDescription {
            kind: "offer".into(),
            sdp: sdp.into(),
        }
    }

    #[test]
    fn accepts_single_audio() {
        let sdp = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=sendonly\r\n";
        assert_eq!(validate_audio_only_offer(&offer(sdp)).unwrap(), "0");
    }

    #[test]
    fn rejects_video_and_multiple_lines() {
        let video = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:0\r\n";
        assert!(validate_audio_only_offer(&offer(video)).is_err());
        let two = "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=audio 9 X 111\r\na=mid:1\r\n";
        assert!(validate_audio_only_offer(&offer(two)).is_err());
        let data = "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:1\r\n";
        assert!(validate_audio_only_offer(&offer(data)).is_err());
        let no_mid = "v=0\r\nm=audio 9 X 111\r\n";
        assert!(validate_audio_only_offer(&offer(no_mid)).is_err());
    }

    #[test]
    fn bridge_offer_needs_the_mid_and_audio_only() {
        let sdp = "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=audio 9 X 111\r\na=mid:1\r\n";
        assert!(validate_audio_offer_mid(&offer(sdp), "1").is_ok());
        assert!(validate_audio_offer_mid(&offer(sdp), "2").is_err());
        let video = "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\nm=video 9 X 96\r\na=mid:1\r\n";
        assert!(validate_audio_offer_mid(&offer(video), "0").is_err());
    }

    #[test]
    fn rejects_answers() {
        let mut d = offer("v=0\r\nm=audio 9 X 111\r\na=mid:0\r\n");
        d.kind = "answer".into();
        assert!(validate_audio_only_offer(&d).is_err());
    }
}
