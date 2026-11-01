//! Misskey's Emoji tags and reaction catalog. Images never bypass the server's DNS/SSRF policy.
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
    if target["remote"] != true {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        let values=query(&mut tx,"SELECT DISTINCT emoji::text FROM apub_reactions WHERE post_id=$1 AND jsonb_typeof(emoji)='object'",&[&input.post_id]).await?;
        return Ok(Json(json!({"software":"zoo","emojis":values})));
    }
    let actor = string(&target, "author")?;
    let origin = reqwest::Url::parse(actor)
        .map_err(internal)?
        .origin()
        .ascii_serialization();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('emoji:'||$1,0))) locked",&[&origin]).await?;
    let cached=query(&mut tx,"SELECT row_to_json(c)::text FROM apub_emoji_catalog c WHERE origin=$1 AND fetched_at>now()-interval '1 hour'",&[&origin]).await?;
    if let Some(c) = cached.first() {
        remember(
            &mut tx,
            c["emojis"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )
        .await?;
        tx.commit().await?;
        return Ok(Json(json!({"software":c["software"],"emojis":c["emojis"]})));
    }
    let mut software = String::new();
    let mut values = vec![];
    if let Ok(discovery) =
        transport::fetch_public_json(&format!("{origin}/.well-known/nodeinfo"), 256 * 1024).await
        && let Some(link) = discovery["links"]
            .as_array()
            .and_then(|links| {
                links.iter().find(|v| {
                    v["rel"]
                        .as_str()
                        .is_some_and(|s| s.ends_with("/2.0") || s.ends_with("/2.1"))
                })
            })
            .and_then(|v| v["href"].as_str())
        && same_origin(link, &origin)
        && let Ok(info) = transport::fetch_public_json(link, 256 * 1024).await
    {
        software = info["software"]["name"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase();
    }
    if matches!(
        software.as_str(),
        "misskey" | "sharkey" | "iceshrimp" | "firefish"
    ) && let Ok(catalog) =
        transport::fetch_public_json(&format!("{origin}/api/emojis"), 8 * 1024 * 1024).await
        && let Some(emojis) = catalog["emojis"].as_array()
    {
        for e in emojis.iter().take(10000) {
            if e["localOnly"] == true
                || e["roleIdsThatCanBeUsedThisEmojiAsReaction"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
            {
                continue;
            }
            let name = e["name"].as_str().unwrap_or("");
            values.extend(tags(&json!({"type":"Emoji","id":format!("{origin}/emojis/{name}"),"name":format!(":{name}:"),"icon":{"url":e["url"]}})));
        }
    }
    // Note tags remain usable when a peer does not expose a catalog.
    let known = query(
        &mut tx,
        "SELECT json_build_object('emojis',emojis)::text FROM apub_remote_posts WHERE id=$1",
        &[&input.post_id],
    )
    .await?;
    if let Some(note) = known.first()
        && let Some(es) = note["emojis"].as_array()
    {
        for e in es {
            if !values.iter().any(|v| v["name"] == e["name"]) {
                values.push(e.clone());
            }
        }
    }
    remember(&mut tx, &values).await?;
    if !software.is_empty()
        && !(matches!(
            software.as_str(),
            "misskey" | "sharkey" | "iceshrimp" | "firefish"
        ) && values.is_empty())
    {
        toasty::sql::statement("INSERT INTO apub_emoji_catalog(origin,software,emojis) VALUES($1,$2,$3::jsonb) ON CONFLICT(origin) DO UPDATE SET software=EXCLUDED.software,emojis=EXCLUDED.emojis,fetched_at=now()").bind(origin).bind(software.as_str()).bind(json!(values).to_string()).exec(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"software":software,"emojis":values})))
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
    let cached = emoji_cache::get(&id, peer, || load_image(url)).await?;
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
        if data.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(AppError::BadGateway("Emoji image too large".into()));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(emoji_cache::Image { mime, bytes: data })
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
