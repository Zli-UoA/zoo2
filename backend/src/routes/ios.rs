use crate::{error::AppError, state::AppState};
use axum::{
    Json, Router,
    http::{HeaderMap, HeaderValue},
    response::Html,
    routing::get,
};
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/.well-known/apple-app-site-association", get(association))
        .route("/ios/oauth/callback", get(callback))
}

pub fn association_document(team_id: &str) -> Result<Value, AppError> {
    if team_id.len() != 10
        || !team_id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(AppError::BadRequest(
            "APPLE_TEAM_ID is not configured".into(),
        ));
    }
    Ok(json!({"webcredentials": {"apps": [format!("{team_id}.works.zli.zoo")]}}))
}

async fn association() -> Result<(HeaderMap, Json<Value>), AppError> {
    let data = association_document(&std::env::var("APPLE_TEAM_ID").unwrap_or_default())?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "cache-control",
        HeaderValue::from_static("public, max-age=300"),
    );
    Ok((headers, Json(data)))
}

async fn callback() -> (HeaderMap, Html<&'static str>) {
    let mut headers = HeaderMap::new();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    // Never reflect the authorization code, state or query string into HTML.
    (
        headers,
        Html(
            "<!doctype html><html lang=ja><meta charset=utf-8><title>Zoo</title><p>Zoo アプリに戻ってログインを完了してください。戻れない場合は、アプリから再度ログインしてください。</p></html>",
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn association_identifies_native_app() {
        let data = association_document("ABCDEFGHIJ").unwrap();
        assert_eq!(
            data["webcredentials"]["apps"][0],
            "ABCDEFGHIJ.works.zli.zoo"
        );
        for invalid in ["", "abc", "abcdefghij", "ABCDEFGHI*"] {
            assert!(association_document(invalid).is_err());
        }
    }
}
