//! Misskey's Emoji tags and received reaction options. Images never bypass the server's DNS/SSRF policy.
use super::*;
use crate::auth::AuthUser;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/activitypub/reaction-options", get(options))
        .route("/api/activitypub/emoji-images/{id}", get(image))
}
fn https(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443)
            && value.len() <= 2048
    })
}
// Actor icons share the bounded, DNS-pinned image proxy/cache used by custom emoji.
pub(super) fn avatar(value: &Value) -> Option<Value> {
    let values = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value));
    values.iter().take(8).find_map(|image| {
        let url = image["url"].as_str()?;
        if !https(url) {
            return None;
        }
        let image_id = openssl::sha::sha256(url.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Some(json!({"url":url,"image_id":image_id}))
    })
}
pub(super) fn media(value: &Value) -> Vec<Value> {
    let values = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value));
    values
        .iter()
        .take(10)
        .filter_map(|item| {
            let mime = item["mediaType"].as_str().unwrap_or("");
            if !matches!(
                mime,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) {
                return None;
            }
            let mut image = avatar(item)?;
            image["media_type"] = json!(mime);
            image["name"] = json!(
                item["name"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(1000)
                    .collect::<String>()
            );
            Some(image)
        })
        .collect()
}
pub(super) fn tags(value: &Value) -> Vec<Value> {
    static NAME: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^:[A-Za-z0-9_+\-]{1,100}:$").unwrap());
    let values: Vec<&Value> = match value {
        Value::Array(v) => v.iter().collect(),
        Value::Object(_) => vec![value],
        _ => vec![],
    };
    values.into_iter().take(128).filter_map(|tag|{
        if tag["type"]!="Emoji"{return None;}
        let name=tag["name"].as_str()?;
        if !NAME.is_match(name){return None;}
        let url=tag["icon"]["url"].as_str()?;
        if !https(url){return None;}
        let image_id=openssl::sha::sha256(url.as_bytes()).iter().map(|b|format!("{b:02x}")).collect::<String>();
        Some(json!({"name":name,"url":url,"id":tag["id"].as_str().filter(|v|https(v)),"image_id":image_id}))
    }).collect()
}
pub(super) async fn remember(
    tx: &mut toasty::Transaction<'_>,
    emojis: &[Value],
) -> Result<(), AppError> {
    toasty::sql::statement("INSERT INTO apub_emoji_images(id,url) SELECT DISTINCT e->>'image_id',e->>'url' FROM jsonb_array_elements($1::jsonb) e ON CONFLICT(id) DO UPDATE SET last_seen_at=now() WHERE apub_emoji_images.last_seen_at<now()-interval '1 day'").bind(json!(emojis).to_string()).exec(&mut *tx).await?;
    toasty::sql::statement(
        "DELETE FROM apub_emoji_images WHERE last_seen_at<now()-interval '30 days'",
    )
    .exec(tx)
    .await?;
    Ok(())
}
pub(super) fn reaction(activity: &Value) -> Result<(String, Option<Value>), AppError> {
    let explicit = activity["_misskey_reaction"].is_string();
    let value = activity["_misskey_reaction"]
        .as_str()
        .or_else(|| activity["content"].as_str())
        .unwrap_or("❤");
    if !explicit && !value.starts_with(':') && emojis::get(value).is_none() {
        return Ok(("❤️".into(), None));
    }
    if value.starts_with(':') {
        let bare = value.trim_matches(':').split('@').next().unwrap_or("");
        let name = format!(":{bare}:");
        let tag = tags(&activity["tag"])
            .into_iter()
            .find(|e| e["name"] == name)
            .ok_or_else(|| {
                AppError::BadRequest("Custom reaction requires a valid Emoji tag".into())
            })?;
        Ok((name, Some(tag)))
    } else if value.chars().count() <= 32 && emojis::get(value).is_some() {
        Ok((emojis::get(value).unwrap().as_str().to_owned(), None))
    } else {
        Err(AppError::BadRequest("Invalid emoji reaction".into()))
    }
}
pub(super) fn as_tag(value: &Value) -> Value {
    json!({"id":value["id"],"type":"Emoji","name":value["name"],"icon":{"type":"Image","url":value["url"]}})
}
#[derive(Deserialize)]
struct Options {
    post_id: String,
}
async fn options(
    State(state): State<AppState>,
    user: AuthUser,
    Query(input): Query<Options>,
) -> Result<Json<Value>, AppError> {
    let target = interactions::target(&state, &user, &input.post_id).await?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let mut values = workspace_options(&mut tx).await?;
    if target["channel_id"].is_null() {
        for descriptor in received_options(&mut tx, &input.post_id).await? {
            if !values
                .iter()
                .any(|e| e["image_id"] == descriptor["image_id"] && e["name"] == descriptor["name"])
            {
                values.push(descriptor);
            }
        }
    }
    Ok(Json(json!({"software":"zoo","emojis":values})))
}

// Only descriptors already received on this authorized target; never fetch a catalog.
pub(super) async fn received_options(
    tx: &mut toasty::Transaction<'_>,
    post_id: &str,
) -> Result<Vec<Value>, AppError> {
    query(tx, "SELECT e::text FROM apub_remote_posts p CROSS JOIN LATERAL jsonb_array_elements(p.emojis) e WHERE p.id=$1 UNION SELECT emoji::text FROM apub_reactions WHERE post_id=$1 AND emoji IS NOT NULL", &[post_id]).await
}

pub(crate) async fn workspace_options(
    tx: &mut toasty::Transaction<'_>,
) -> Result<Vec<Value>, AppError> {
    let rows = query(
        tx,
        "SELECT row_to_json(e)::text FROM workspace_emojis e ORDER BY name",
        &[],
    )
    .await?;
    rows.into_iter().map(|e| {
        let url = string(&e, "url")?;
        let url = if url.starts_with('/') { format!("{}{url}", base_url().unwrap_or_default()) } else { url.to_owned() };
        let image_id = openssl::sha::sha256(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        Ok(json!({"name":format!(":{}:",string(&e,"name")?),"url":url,"id":url,"image_id":image_id,"local":true,"category":e["category"],"aliases":e["aliases"]}))
    }).collect()
}

async fn image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> Result<Response, AppError> {
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|v| v.0.ip());
    let peer = client_ip::resolve(
        peer,
        request.headers(),
        matches!(
            std::env::var("ACTIVITYPUB_CLOUDFLARE_TUNNEL").as_deref(),
            Ok("true" | "1")
        ),
    )?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let image = one(
        &mut tx,
        "SELECT json_build_object('url',url)::text FROM apub_emoji_images WHERE id=$1",
        &[&id],
    )
    .await?;
    drop(tx);
    let url = reqwest::Url::parse(string(&image, "url")?).map_err(internal)?;
    let cached = match emoji_cache::get(&id, peer, || load_image(url)).await {
        Err(AppError::RateLimited) => {
            return Ok((
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                [
                    (header::RETRY_AFTER, "60"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                Json(json!({"error":"Rate limit exceeded"})),
            )
                .into_response());
        }
        result => result?,
    };
    Ok(([(header::CONTENT_TYPE,cached.mime.as_str()),(header::CACHE_CONTROL,"public, max-age=300"),(header::X_CONTENT_TYPE_OPTIONS,"nosniff"),(header::CONTENT_SECURITY_POLICY,"sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:; frame-ancestors 'none'")],cached.bytes).into_response())
}
async fn load_image(url: reqwest::Url) -> Result<emoji_cache::Image, AppError> {
    let mut response = transport::safe_client(&url)
        .await?
        .get(url)
        .header(header::ACCEPT, "image/*")
        .send()
        .await
        .map_err(|_| AppError::BadGateway("Emoji image request failed".into()))?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway("Emoji image unavailable".into()));
    }
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_owned();
    if !matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" | "image/svg+xml"
    ) {
        return Err(AppError::BadGateway("Unsupported emoji image".into()));
    }
    let mut data = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::BadGateway("Emoji image transfer failed".into()))?
    {
        // Home photos share this DNS-pinned proxy and the 32 MiB total cache budget.
        if data.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(AppError::BadGateway("Emoji image too large".into()));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(emoji_cache::Image { mime, bytes: data })
}

pub(super) fn note_content(note: &Value) -> Result<String, AppError> {
    let html = string(note, "content")?;
    let source = (note["source"]["mediaType"] == "text/x.misskeymarkdown")
        .then(|| note["source"]["content"].as_str())
        .flatten()
        .or_else(|| note["_misskey_content"].as_str());
    if html.len() > 128 * 1024 || source.is_some_and(|value| value.len() > 128 * 1024) {
        return Err(AppError::BadRequest("Note too large".into()));
    }
    Ok(source
        .map(str::to_owned)
        .unwrap_or_else(|| plain_content(html)))
}

pub(super) fn plain_content(raw: &str) -> String {
    let alt = regex::Regex::new(r#"(?is)<img\b[^>]*\balt=["']([^"']*)["'][^>]*>"#)
        .unwrap()
        .replace_all(raw, "$1");
    let lines = regex::Regex::new("(?i)<br\\s*/?>|</p>")
        .unwrap()
        .replace_all(&alt, "\n");
    let plain = regex::Regex::new("<[^>]*>")
        .unwrap()
        .replace_all(&lines, "");
    html_escape::decode_html_entities(&plain).into_owned()
}

#[cfg(test)]
mod avatar_tests {
    use super::*;
    #[test]
    fn actor_icons_use_registered_https_image_ids() {
        let icon = json!({"type":"Image","url":"https://cdn.peer.example/avatar.png"});
        let value = avatar(&icon).unwrap();
        assert_eq!(value["url"], icon["url"]);
        assert_eq!(value["image_id"].as_str().unwrap().len(), 64);
        assert_eq!(avatar(&json!([icon])), Some(value));
        for url in [
            "http://peer.example/icon.png",
            "data:image/png;base64,eA==",
            "javascript:alert(1)",
            "https://user:password@peer.example/icon.png",
            "https://peer.example:8443/icon.png",
        ] {
            assert!(avatar(&json!({"url":url})).is_none());
        }
        assert!(avatar(&Value::Null).is_none());
    }
}
