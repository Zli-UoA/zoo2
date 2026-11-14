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
    async fn send_e2ee(
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
