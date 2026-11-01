//! HTTP Signature canonicalization is provided by http-signature-normalization.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use http_signature_normalization::Config;
use openssl::{
    hash::MessageDigest,
    pkey::PKey,
    sign::{Signer, Verifier},
};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};

pub fn digest(body: &[u8]) -> String {
    format!("SHA-256={}", STANDARD.encode(openssl::sha::sha256(body)))
}
fn signature_config() -> Config {
    Config::new()
        .mastodon_compat()
        .require_header("(request-target)")
        .set_expiration(Duration::from_secs(300))
}
pub fn sign_headers(
    method: &str,
    url: &reqwest::Url,
    body: Option<&[u8]>,
    actor: &str,
    pem: &str,
) -> Result<BTreeMap<String, String>, AppError> {
    let mut headers = BTreeMap::from([
        ("host".into(), url.authority().to_string()),
        ("date".into(), httpdate::fmt_http_date(SystemTime::now())),
    ]);
    if let Some(body) = body {
        headers.insert("digest".into(), digest(body));
    }
    let config = if body.is_some() {
        signature_config().require_digest()
    } else {
        signature_config()
    };
    let signed = config
        .begin_sign(method, &target(url), headers.clone())
        .map_err(internal)?
        .sign(
            format!("{actor}#main-key"),
            |text| -> Result<String, AppError> {
                let key = PKey::private_key_from_pem(pem.as_bytes()).map_err(internal)?;
                let mut signer = Signer::new(MessageDigest::sha256(), &key).map_err(internal)?;
                signer.update(text.as_bytes()).map_err(internal)?;
                Ok(STANDARD.encode(signer.sign_to_vec().map_err(internal)?))
            },
        )?;
    headers.insert("signature".into(), signed.signature_header());
    Ok(headers)
}
fn target(url: &reqwest::Url) -> String {
    match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().into(),
    }
}
pub fn verify_headers(
    headers: &axum::http::HeaderMap,
    target: &str,
    body: &[u8],
    actor: &Value,
) -> Result<(), AppError> {
    let invalid = || AppError::Unauthorized("Invalid ActivityPub HTTP signature".into());
    let signature = validate_envelope(headers, target, body)?;
    if signature.key_id() != string(actor, "key_id")? {
        return Err(invalid());
    }
    let key = PKey::public_key_from_pem(string(actor, "public_key")?.as_bytes())
        .map_err(|_| invalid())?;
    let signature_bytes = STANDARD
        .decode(signature.signature())
        .map_err(|_| invalid())?;
    let mut verifier = Verifier::new(MessageDigest::sha256(), &key).map_err(internal)?;
    verifier
        .update(signature.signing_string().as_bytes())
        .map_err(internal)?;
    if !verifier.verify(&signature_bytes).map_err(|_| invalid())? {
        return Err(invalid());
    }
    Ok(())
}

pub async fn safe_client(url: &reqwest::Url) -> Result<reqwest::Client, AppError> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(AppError::BadRequest(
            "Federation requires an HTTPS URL on port 443".into(),
        ));
    }
    let host = url.host_str().unwrap();
    let addresses: Vec<_> =
        tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
            .await
            .map_err(|_| AppError::BadGateway("Federation DNS lookup timed out".into()))?
            .map_err(|_| AppError::BadGateway("Federation DNS lookup failed".into()))?
            .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|a| !crate::apps::delivery::public_ip(a.ip()))
    {
        return Err(AppError::BadRequest(
            "Federation endpoint must resolve to public addresses".into(),
        ));
    }
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(internal)
}
pub async fn request(
    state: &AppState,
    user: Option<&str>,
    method: reqwest::Method,
    raw_url: &str,
    payload: Option<&Value>,
) -> Result<reqwest::Response, AppError> {
    let url = reqwest::Url::parse(raw_url)
        .map_err(|_| AppError::BadRequest("Invalid federation URL".into()))?;
    let client = safe_client(&url).await?;
    request_with_client(state, user, method, &url, payload, &client).await
}
// Keep the signed request builder shared with the loopback transport test.
pub(super) async fn request_with_client(
    state: &AppState,
    user: Option<&str>,
    method: reqwest::Method,
    url: &reqwest::Url,
    payload: Option<&Value>,
    client: &reqwest::Client,
) -> Result<reqwest::Response, AppError> {
    let body = payload.map(Value::to_string);
    let mut req = client
        .request(method.clone(), url.clone())
        .header("Accept", CONTENT_TYPE)
        .header("User-Agent", "Zoo-ActivityPub/1.0");
    if let Some(user) = user {
        let key = keys(state, user).await?;
        for (k, v) in sign_headers(
            method.as_str(),
            url,
            body.as_deref().map(str::as_bytes),
            &actor_id(user)?,
            string(&key, "private_key")?,
        )? {
            req = req.header(k, v);
        }
    }
    if let Some(body) = body {
        req = req.header("Content-Type", CONTENT_TYPE).body(body);
    }
    req.send()
        .await
        .map_err(|_| AppError::BadGateway("Federation request failed".into()))
}
pub async fn fetch_json(
    state: &AppState,
    user: Option<&str>,
    url: &str,
) -> Result<Value, AppError> {
    fetch_json_limit(state, user, url, 256 * 1024).await
}
pub(super) async fn fetch_json_limit(
    state: &AppState,
    user: Option<&str>,
    url: &str,
    limit: usize,
) -> Result<Value, AppError> {
    let response = request(state, user, reqwest::Method::GET, url, None).await?;
    read_json(response, limit).await
}
pub(super) async fn fetch_public_json(url: &str, limit: usize) -> Result<Value, AppError> {
    let url = reqwest::Url::parse(url).map_err(internal)?;
    let response = safe_client(&url)
        .await?
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "Zoo-ActivityPub/1.0")
        .send()
        .await
        .map_err(internal)?;
    read_json(response, limit).await
}
async fn read_json(mut response: reqwest::Response, limit: usize) -> Result<Value, AppError> {
    if !response.status().is_success() {
        return Err(AppError::BadGateway("Remote resource unavailable".into()));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(internal)? {
        if data.len() + chunk.len() > limit {
            return Err(AppError::BadGateway("Remote document too large".into()));
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).map_err(|_| AppError::BadGateway("Invalid remote JSON".into()))
}

pub fn validate_envelope(
    headers: &axum::http::HeaderMap,
    target: &str,
    body: &[u8],
) -> Result<http_signature_normalization::verify::Unverified, AppError> {
    let invalid = || AppError::Unauthorized("Invalid ActivityPub HTTP signature".into());
    let date = headers
        .get("date")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| httpdate::parse_http_date(v).ok())
        .ok_or_else(invalid)?;
    if SystemTime::now()
        .duration_since(date)
        .unwrap_or_else(|e| e.duration())
        > Duration::from_secs(300)
    {
        return Err(invalid());
    }
    let expected = openssl::sha::sha256(body);
    let supplied = headers
        .get("digest")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("SHA-256="))
        .and_then(|v| STANDARD.decode(v).ok())
        .ok_or_else(invalid)?;
    if supplied.len() != expected.len() || !openssl::memcmp::eq(&expected, &supplied) {
        return Err(invalid());
    }
    let map = headers
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect();
    let signature = signature_config()
        .require_digest()
        .begin_verify("POST", target, map)
        .map_err(|_| invalid())?;
    Ok(signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_bind_body_target_and_key() -> anyhow::Result<()> {
        let key = PKey::from_rsa(openssl::rsa::Rsa::generate(2048)?)?;
        let private = String::from_utf8(key.private_key_to_pem_pkcs8()?)?;
        let public = String::from_utf8(key.public_key_to_pem()?)?;
        let url = reqwest::Url::parse("https://remote.example/inbox?key=value")?;
        let actor = "https://zoo.example/users/alice";
        let body = br#"{"type":"Follow"}"#;
        let signed = sign_headers("POST", &url, Some(body), actor, &private)?;
        let headers: axum::http::HeaderMap = signed
            .iter()
            .map(|(k, v)| {
                Ok((
                    axum::http::HeaderName::from_bytes(k.as_bytes())?,
                    axum::http::HeaderValue::from_str(v)?,
                ))
            })
            .collect::<Result<_, anyhow::Error>>()?;
        let actor = json!({"key_id":format!("{actor}#main-key"),"public_key":public});
        assert!(verify_headers(&headers, "/inbox?key=value", body, &actor).is_ok());
        assert!(verify_headers(&headers, "/other", body, &actor).is_err());
        assert!(verify_headers(&headers, "/inbox?key=value", b"tampered", &actor).is_err());
        let mut invalid = headers.clone();
        invalid.insert(
            "digest",
            axum::http::HeaderValue::from_static("SHA-256=AA=="),
        );
        assert!(verify_headers(&invalid, "/inbox?key=value", body, &actor).is_err());
        let mut expired = headers.clone();
        expired.insert(
            "date",
            axum::http::HeaderValue::from_static("Thu, 01 Jan 1970 00:00:00 GMT"),
        );
        assert!(verify_headers(&expired, "/inbox?key=value", body, &actor).is_err());
        Ok(())
    }
    #[tokio::test]
    async fn rejects_non_public_destinations() {
        for url in [
            "https://127.0.0.1/inbox",
            "https://10.0.0.1/inbox",
            "http://example.com/inbox",
            "https://example.com:8443/inbox",
            "https://user:pass@example.com/inbox",
        ] {
            assert!(
                safe_client(&reqwest::Url::parse(url).unwrap())
                    .await
                    .is_err(),
                "{url}"
            );
        }
    }
}
