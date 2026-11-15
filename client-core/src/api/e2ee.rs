//! 初回設定と解錠で使う E2EE の API（docs/e2ee.md の「クロス署名」「Secret Storage」「鍵バックアップ」。
//! #360）。React の `secretStorageApi.ts` と同じ。
//!
//! 直前の再認証が要る要求には、`reauth` に再認証で受け取った ID トークンを渡す（`X-Zoo-Reauth`）。
//! エラーは [`crate::http::ApiError`] にして、`code`（`E2EE_REAUTH_REQUIRED` など）で分岐できるようにする。

use anyhow::Result;
use reqwest::Method;
use serde_json::{Value, json};

use super::{ApiClient, Envelope};
use crate::http::{ApiError, error_for_status_coded};

/// 直前の再認証の ID トークンを入れるヘッダー。
pub const REAUTH_HEADER: &str = "X-Zoo-Reauth";

impl ApiClient {
    /// E2EE の要求を送る。エラーは [`ApiError`]。
    pub(super) async fn send_e2ee(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        reauth: Option<&str>,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(Self::E2EE_TIMEOUT);
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(reauth) = reauth {
            request = request.header(REAUTH_HEADER, reauth);
        }
        let request = self.credentials.apply(request).await?;
        error_for_status_coded(request.send().await?).await
    }

    /// 封筒なしの JSON を返す要求（GET と `keys/query`）。
    async fn e2ee_json(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        Ok(self
            .send_e2ee(method, path, body, None)
            .await?
            .json()
            .await?)
    }

    /// 書き込みの要求。封筒 `{data, txid}` の `data` を返す。
    async fn e2ee_write(&self, path: &str, body: Value, reauth: Option<&str>) -> Result<Value> {
        let envelope: Envelope = self
            .send_e2ee(Method::POST, path, Some(body), reauth)
            .await?
            .json()
            .await?;
        Ok(envelope.data.unwrap_or(Value::Null))
    }

    /// `GET /api/e2ee/secret_storage`。SSSS がまだなければ `default_key_id` が `null`。
    pub async fn e2ee_secret_storage(&self) -> Result<Value> {
        self.e2ee_json(Method::GET, "/api/e2ee/secret_storage", None)
            .await
    }

    /// `POST /api/e2ee/secret_storage/keys`。増やしたあとの `secrets_revision` を返す。
    pub async fn e2ee_create_secret_storage_key(
        &self,
        body: Value,
        reauth: Option<&str>,
    ) -> Result<Option<u64>> {
        let data = self
            .e2ee_write("/api/e2ee/secret_storage/keys", body, reauth)
            .await?;
        Ok(data.get("secrets_revision").and_then(Value::as_u64))
    }

    /// `POST /api/e2ee/secret_storage/slots`。`label` が `null` なら送らない（サーバーは `null` を受け付けない）。
    pub async fn e2ee_add_key_slot(&self, slot: Value, reauth: Option<&str>) -> Result<()> {
        let mut slot = slot;
        if let Some(object) = slot.as_object_mut()
            && object.get("label").is_some_and(Value::is_null)
        {
            object.remove("label");
        }
        self.e2ee_write("/api/e2ee/secret_storage/slots", slot, reauth)
            .await?;
        Ok(())
    }

    /// `POST /api/e2ee/secret_storage/slots/{slot_id}/used`。表示用の記録なので、失敗は無視する。
    pub async fn e2ee_mark_key_slot_used(&self, slot_id: &str) {
        let path = format!("/api/e2ee/secret_storage/slots/{}/used", urlencode(slot_id));
        let _ = self.e2ee_write(&path, json!({}), None).await;
    }

    /// 自分だけを問い合わせた `POST /api/e2ee/keys/query`（クロス署名鍵と署名を含む、封筒なしの応答）。
    pub async fn e2ee_query_own_keys(&self, user_id: &str) -> Result<Value> {
        self.e2ee_json(
            Method::POST,
            "/api/e2ee/keys/query",
            Some(json!({ "user_ids": [user_id] })),
        )
        .await
    }

    /// `GET /api/e2ee/key_backup/versions`（消した版を含むすべての版）。
    pub async fn e2ee_backup_versions(&self) -> Result<Value> {
        self.e2ee_json(Method::GET, "/api/e2ee/key_backup/versions", None)
            .await
    }

    /// `GET /api/e2ee/key_backup/versions/current`。今の版がなければ `None`。
    pub async fn e2ee_current_backup_version(&self) -> Result<Option<Value>> {
        match self
            .e2ee_json(Method::GET, "/api/e2ee/key_backup/versions/current", None)
            .await
        {
            Ok(value) => Ok(Some(value)),
            Err(error) if ApiError::status_of(&error) == Some(reqwest::StatusCode::NOT_FOUND) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// `POST /api/e2ee/cross_signing/keys`。
    pub async fn e2ee_upload_cross_signing_keys(
        &self,
        body: Value,
        reauth: Option<&str>,
    ) -> Result<()> {
        self.e2ee_write("/api/e2ee/cross_signing/keys", body, reauth)
            .await?;
        Ok(())
    }

    /// `POST /api/e2ee/cross_signing/signatures`。
    pub async fn e2ee_upload_signatures(&self, signatures: Value) -> Result<()> {
        self.e2ee_write(
            "/api/e2ee/cross_signing/signatures",
            json!({ "signatures": signatures }),
            None,
        )
        .await?;
        Ok(())
    }

    /// `POST /api/e2ee/key_backup/versions`。作った（または同じ公開鍵の今の）版を返す。
    pub async fn e2ee_create_backup_version(
        &self,
        body: Value,
        reauth: Option<&str>,
    ) -> Result<Option<String>> {
        let data = self
            .e2ee_write("/api/e2ee/key_backup/versions", body, reauth)
            .await?;
        Ok(data
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// `GET /api/e2ee/key_backup/keys?version=<版>&after=<前のページの next>`（1000 件ずつ）。
    /// `{"sessions": [...], "next": <続きのカーソルか null>}` を返す。
    pub async fn e2ee_backup_keys(&self, version: &str, after: Option<&str>) -> Result<Value> {
        let mut path = format!("/api/e2ee/key_backup/keys?version={}", urlencode(version));
        if let Some(after) = after {
            path.push_str(&format!("&after={}", urlencode(after)));
        }
        self.e2ee_json(Method::GET, &path, None).await
    }

    /// `POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`。
    pub async fn e2ee_upload_backup_keys(
        &self,
        version: &str,
        secrets_revision: u64,
        body: Value,
    ) -> Result<()> {
        let path = format!(
            "/api/e2ee/key_backup/keys?version={}&secrets_revision={secrets_revision}",
            urlencode(version)
        );
        self.e2ee_write(&path, body, None).await?;
        Ok(())
    }
}

/// to-device の宛先の端末（キューの上限で入れなかった宛先など）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceTarget {
    pub user_id: String,
    pub device_id: String,
}

/// `keys/claim` で一度に払い出す端末の数（サーバーの上限）。
pub const KEYS_CLAIM_BATCH: usize = 1000;
/// `to_device` の 1 回の宛先の数（サーバーの上限）。
pub const TO_DEVICE_BATCH: usize = 100;

/// 暗号化チャンネルの送信に使う API（docs/e2ee.md の「配布先のスナップショット」「端末とワンタイム鍵」
/// 「to-device メッセージ」「クロス署名」「メッセージ」。#361）。React の `megolmApi.ts` と同じ。
impl ApiClient {
    /// `GET /api/e2ee/channels/{channel_id}/recipients` の 1 ページ（封筒なしの JSON）。
    /// 続きは `after`（前のページの `next_after`）と `epoch`（最初のページの `membership_epoch`）で取る。
    /// ページの途中で `membership_epoch` が変わると 409 `E2EE_STALE_EPOCH`。
    pub async fn e2ee_channel_recipients(
        &self,
        channel_id: &str,
        after: Option<&str>,
        epoch: Option<&str>,
    ) -> Result<Value> {
        let mut query = Vec::new();
        if let Some(after) = after {
            query.push(format!("after={}", urlencode(after)));
        }
        if let Some(epoch) = epoch {
            query.push(format!("epoch={}", urlencode(epoch)));
        }
        let query = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        let path = format!(
            "/api/e2ee/channels/{}/recipients{query}",
            urlencode(channel_id)
        );
        self.e2ee_json(Method::GET, &path, None).await
    }

    /// `POST /api/e2ee/tofu_pins`（`replace: false`）でピンを 1 件置く。409 `E2EE_PIN_CONFLICT`
    /// （別の端末が先に置いた）なら `false`。
    pub async fn e2ee_post_tofu_pin(&self, pin: &Value) -> Result<bool> {
        match self
            .e2ee_write(
                "/api/e2ee/tofu_pins",
                json!({ "pins": [pin], "replace": false }),
                None,
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if ApiError::has_code(&error, "E2EE_PIN_CONFLICT") => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// `POST /api/e2ee/keys/claim`（[`KEYS_CLAIM_BATCH`] 台ずつ）。`claim` は
    /// `{user_id: [device_id]}`。応答の `one_time_keys`（`{user_id: {device_id: {key_id: 鍵}}}`）をまとめて返す。
    pub async fn e2ee_claim_keys(
        &self,
        claim: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> Result<serde_json::Map<String, Value>> {
        let entries: Vec<(&String, &String)> = claim
            .iter()
            .flat_map(|(user_id, devices)| devices.iter().map(move |device| (user_id, device)))
            .collect();
        let mut claimed = serde_json::Map::new();
        for chunk in entries.chunks(KEYS_CLAIM_BATCH) {
            let mut batch = serde_json::Map::new();
            for (user_id, device_id) in chunk {
                let devices = batch
                    .entry((*user_id).clone())
                    .or_insert_with(|| Value::Object(Default::default()));
                devices[device_id.as_str()] = json!("signed_curve25519");
            }
            let data = self
                .e2ee_write(
                    "/api/e2ee/keys/claim",
                    json!({ "one_time_keys": batch }),
                    None,
                )
                .await?;
            if let Some(Value::Object(users)) = data.get("one_time_keys") {
                for (user_id, devices) in users {
                    let Value::Object(devices) = devices else {
                        continue;
                    };
                    let entry = claimed
                        .entry(user_id.clone())
                        .or_insert_with(|| Value::Object(Default::default()));
                    if let Value::Object(entry) = entry {
                        entry.extend(devices.clone());
                    }
                }
            }
        }
        Ok(claimed)
    }

    /// `POST /api/e2ee/to_device`（[`TO_DEVICE_BATCH`] 端末ずつ、要求ごとに別の `txn_id`）。`messages` は
    /// `{user_id: {device_id: content}}`。キューの上限で入れなかった宛先（`undelivered`）を返す。
    pub async fn e2ee_send_to_device(
        &self,
        device_id: &str,
        messages: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, Value>>,
    ) -> Result<Vec<DeviceTarget>> {
        let entries: Vec<(&String, &String, &Value)> = messages
            .iter()
            .flat_map(|(user_id, devices)| {
                devices
                    .iter()
                    .map(move |(target, content)| (user_id, target, content))
            })
            .collect();
        let mut undelivered = Vec::new();
        for chunk in entries.chunks(TO_DEVICE_BATCH) {
            let mut batch = serde_json::Map::new();
            for (user_id, target, content) in chunk {
                let devices = batch
                    .entry((*user_id).clone())
                    .or_insert_with(|| Value::Object(Default::default()));
                devices[target.as_str()] = (*content).clone();
            }
            let data = self
                .e2ee_write(
                    "/api/e2ee/to_device",
                    json!({
                        "device_id": device_id,
                        "txn_id": uuid::Uuid::new_v4().to_string(),
                        "type": "m.room.encrypted",
                        "messages": batch,
                    }),
                    None,
                )
                .await?;
            for target in data
                .get("undelivered")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let (Some(user_id), Some(device_id)) = (
                    target.get("user_id").and_then(Value::as_str),
                    target.get("device_id").and_then(Value::as_str),
                ) {
                    undelivered.push(DeviceTarget {
                        user_id: user_id.to_owned(),
                        device_id: device_id.to_owned(),
                    });
                }
            }
        }
        Ok(undelivered)
    }

    /// 暗号化チャンネルにメッセージを送る（`POST /api/messages`。`content` は空、`ciphertext` に暗号文）。
    /// `body` は要求の全体。エラーは [`ApiError`]（409 `E2EE_STALE_EPOCH` の `current_epoch` を読むため）。
    pub async fn send_encrypted_message(&self, body: Value) -> Result<Option<String>> {
        let envelope: Envelope = self
            .send_e2ee(Method::POST, "/api/messages", Some(body), None)
            .await?
            .json()
            .await?;
        Ok(envelope.txid)
    }

    /// 暗号化チャンネルのメッセージを編集する（`PATCH /api/messages/{id}`。`{content: "", epoch, revision, ciphertext}`）。
    pub async fn edit_encrypted_message(&self, id: &str, body: Value) -> Result<Option<String>> {
        let path = format!("/api/messages/{}", urlencode(id));
        let envelope: Envelope = self
            .send_e2ee(Method::PATCH, &path, Some(body), None)
            .await?
            .json()
            .await?;
        Ok(envelope.txid)
    }
}

/// 409 `E2EE_STALE_EPOCH` の `current_epoch`。違うエラーなら `None`。
pub fn stale_epoch(error: &anyhow::Error) -> Option<u64> {
    let api = error.downcast_ref::<ApiError>()?;
    if api.code.as_deref() != Some("E2EE_STALE_EPOCH") {
        return None;
    }
    json_u64(api.body.get("current_epoch")?)
}

/// 409 `E2EE_STALE_REVISION` の `current_revision`。違うエラーなら `None`。
pub fn stale_revision(error: &anyhow::Error) -> Option<u64> {
    let api = error.downcast_ref::<ApiError>()?;
    if api.code.as_deref() != Some("E2EE_STALE_REVISION") {
        return None;
    }
    json_u64(api.body.get("current_revision")?)
}

/// 数か、数の文字列（int8 は文字列で届くことがある）。
pub fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    use super::*;
    use crate::http::Credentials;

    fn client(server: &MockServer) -> ApiClient {
        ApiClient::new(
            reqwest::Client::new(),
            &server.uri(),
            Credentials::Dev("00000000-0000-0000-0000-000000000001".into()),
        )
    }

    #[tokio::test]
    async fn sends_the_reauth_header_and_reads_error_codes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/e2ee/secret_storage/keys"))
            .and(header(REAUTH_HEADER, "id-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": {"secrets_revision": 1}, "txid": "1"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/e2ee/secret_storage/keys"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": "Re-authentication required", "code": "E2EE_REAUTH_REQUIRED",
            })))
            .mount(&server)
            .await;
        let api = client(&server);
        assert_eq!(
            api.e2ee_create_secret_storage_key(json!({}), Some("id-token"))
                .await
                .unwrap(),
            Some(1)
        );
        let err = api
            .e2ee_create_secret_storage_key(json!({}), None)
            .await
            .unwrap_err();
        assert!(ApiError::has_code(&err, "E2EE_REAUTH_REQUIRED"));
        assert_eq!(
            ApiError::status_of(&err),
            Some(reqwest::StatusCode::FORBIDDEN)
        );
    }

    #[tokio::test]
    async fn current_backup_version_is_none_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/e2ee/key_backup/versions/current"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "none"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/e2ee/key_backup/keys"))
            .and(query_param("version", "3"))
            .and(query_param("secrets_revision", "7"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": {"count": 0}, "txid": null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let api = client(&server);
        assert!(api.e2ee_current_backup_version().await.unwrap().is_none());
        api.e2ee_upload_backup_keys("3", 7, json!({"sessions": []}))
            .await
            .unwrap();
    }
}
