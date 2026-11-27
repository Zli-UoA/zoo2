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
        .route("/api/activitypub/media-images/{id}", get(media_image))
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
pub(super) fn note_media(note: &Value) -> Vec<Value> {
    let mut images = media(&note["attachment"]);
    if note["sensitive"] == true {
        for image in &mut images {
            image["is_sensitive"] = json!(true);
        }
    }
    images
}
fn needs_media_blurhash(item: &Value) -> bool {
    item["image_id"].is_string() && item["blurhash"].is_null()
}

async fn fill_media_blurhashes(media: &mut [Value], only: Option<&[Value]>) -> (bool, Vec<String>) {
    let mut images = std::collections::BTreeMap::new();
    for item in media.iter().filter(|item| needs_media_blurhash(item)) {
        if let (Some(id), Some(url)) = (item["image_id"].as_str(), item["url"].as_str())
            && only.is_none_or(|ids| ids.iter().any(|value| value.as_str() == Some(id)))
        {
            images.insert(id.to_owned(), url.to_owned());
        }
    }
    let hashes = futures_util::future::join_all(images.into_iter().map(|(id, url)| async move {
        // Optional metadata must not keep an otherwise valid Note from being delivered.
        let hash = tokio::time::timeout(Duration::from_secs(5), async {
            let Ok(url) = reqwest::Url::parse(&url) else {
                return Ok(None);
            };
            match emoji_cache::get(&format!("media:{id}"), None, || load_image(url, true)).await {
                Ok(image) => Ok(image.blurhash),
                Err(
                    AppError::RateLimited
                    | AppError::BadGateway(_)
                    | AppError::ServiceUnavailable(_)
                    | AppError::Internal(_),
                ) => Err(()),
                Err(_) => Ok(None),
            }
        })
        .await
        .unwrap_or(Err(()));
        (id, hash)
    }))
    .await
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    let mut changed = false;
    for item in media.iter_mut().filter(|item| needs_media_blurhash(item)) {
        if let Some(hash) = item["image_id"]
            .as_str()
            .and_then(|id| hashes.get(id))
            .and_then(|result| result.as_ref().ok())
            .and_then(Option::as_deref)
        {
            item["blurhash"] = json!(hash);
            changed = true;
        }
    }
    let retry = hashes
        .into_iter()
        .filter_map(|(id, result)| result.is_err().then_some(id))
        .collect();
    (changed, retry)
}

pub(super) async fn queue_media_blurhash(
    tx: &mut toasty::Transaction<'_>,
    post_id: &str,
    media: &[Value],
) -> Result<(), AppError> {
    if media.iter().any(needs_media_blurhash) {
        toasty::sql::statement(
            "INSERT INTO apub_media_blurhash_queue(post_id) VALUES($1)
             ON CONFLICT(post_id) DO UPDATE SET available_at=now(), lease_id=NULL, attempts=0, retry_image_ids=NULL",
        )
        .bind(post_id)
        .exec(tx)
        .await?;
    }
    Ok(())
}

pub(super) async fn blurhash_worker(state: &AppState) {
    loop {
        match complete_next_media_blurhash(state).await {
            Ok(true) => {}
            Ok(false) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(error) => {
                tracing::warn!(%error, "ActivityPub image BlurHash worker failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn complete_next_media_blurhash(state: &AppState) -> Result<bool, AppError> {
    let mut db = state.db.clone();
    let lease = uuid::Uuid::new_v4().to_string();
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        "WITH claimed AS (
             UPDATE apub_media_blurhash_queue SET available_at=now()+interval '30 seconds', lease_id=$1, attempts=attempts+1
             WHERE post_id=(SELECT post_id FROM apub_media_blurhash_queue WHERE available_at<=now()
               ORDER BY available_at,post_id LIMIT 1 FOR UPDATE SKIP LOCKED)
             RETURNING post_id,attempts,retry_image_ids
         )
         SELECT json_build_object('id',p.id,'actor_id',p.actor_id,'updated_at',p.updated_at,
             'active',p.deleted_at IS NULL,'media',p.media,'attempts',claimed.attempts,'retry_image_ids',claimed.retry_image_ids)::text
         FROM claimed JOIN apub_remote_posts p ON p.id=claimed.post_id",
        &[&lease],
    )
    .await?;
    tx.commit().await?;
    let Some(job) = rows.first() else {
        return Ok(false);
    };
    let post_id = string(job, "id")?;
    let mut retry = Vec::new();
    if job["active"] == true
        && let Some(items) = job["media"].as_array()
    {
        let mut media = items.clone();
        let (changed, retry_images) = fill_media_blurhashes(
            &mut media,
            job["retry_image_ids"].as_array().map(Vec::as_slice),
        )
        .await;
        retry = retry_images;
        if changed {
            let updated = string(job, "updated_at")?
                .parse::<jiff::Timestamp>()
                .map_err(internal)?;
            store_blurhashes(
                &mut db,
                post_id,
                string(job, "actor_id")?,
                updated,
                &job["media"].to_string(),
                &json!(media).to_string(),
            )
            .await?;
        }
    }
    if !retry.is_empty()
        && job["attempts"]
            .as_u64()
            .is_some_and(|attempts| attempts < 3)
    {
        toasty::sql::statement(
            "UPDATE apub_media_blurhash_queue SET lease_id=NULL, retry_image_ids=$3::jsonb, available_at=now()+
             CASE WHEN attempts=1 THEN interval '30 seconds' ELSE interval '2 minutes' END
             WHERE post_id=$1 AND lease_id=$2",
        )
        .bind(post_id).bind(&lease).bind(json!(retry).to_string()).exec(&mut db).await?;
    } else {
        toasty::sql::statement(
            "DELETE FROM apub_media_blurhash_queue WHERE post_id=$1 AND lease_id=$2",
        )
        .bind(post_id)
        .bind(&lease)
        .exec(&mut db)
        .await?;
    }
    Ok(true)
}

async fn store_blurhashes(
    db: &mut toasty::Db,
    post_id: &str,
    actor_id: &str,
    updated: jiff::Timestamp,
    original: &str,
    enriched: &str,
) -> Result<(), toasty::Error> {
    // The primary key limits this write to one row. Check both the timestamp
    // and attachment snapshot because a canonical refresh can reuse the timestamp.
    toasty::sql::statement(
        "UPDATE apub_remote_posts SET media=$5::jsonb WHERE id=$1 AND actor_id=$2
         AND updated_at=$3::timestamptz AND deleted_at IS NULL AND media=$4::jsonb",
    )
    .bind(post_id)
    .bind(actor_id)
    .bind(updated.to_string())
    .bind(original)
    .bind(enriched)
    .exec(db)
    .await?;
    Ok(())
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
            let mime = item["mediaType"]
                .as_str()
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let mime = mime.as_str();
            let document = mime.starts_with("text/")
                || mime.starts_with("application/vnd.openxmlformats-officedocument.")
                || mime.starts_with("application/vnd.oasis.opendocument.")
                || matches!(
                    mime,
                    "message/rfc822"
                        | "application/vnd.ms-outlook"
                        | "application/zip"
                        | "application/x-zip-compressed"
                        | "application/json"
                        | "application/xml"
                );
            if (!crate::storage::is_inline_allowed_mime(mime) && !document) || mime == "image/avif"
            {
                return None;
            }
            let mut image = avatar(item)?;
            // Only raster images use the size-limited image proxy. Other media are
            // played directly by the browser and must not enter the image registry.
            if !mime.starts_with("image/") {
                image.as_object_mut()?.remove("image_id");
            }
            image["media_type"] = json!(mime);
            image["is_sensitive"] = json!(item["sensitive"] == true);
            image["name"] = json!(
                item["name"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(1000)
                    .collect::<String>()
            );
            for key in ["width", "height"] {
                if let Some(size) = item[key]
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= i32::MAX as u64)
                {
                    image[key] = json!(size);
                }
            }
            if let Some(hash) = item["blurhash"]
                .as_str()
                .filter(|hash| valid_blurhash(hash))
            {
                image["blurhash"] = json!(hash);
            }
            Some(image)
        })
        .collect()
}
// BlurHash uses base83 and 1..=9 components on each axis (6..=166 bytes).
fn valid_blurhash(hash: &str) -> bool {
    const BASE83: &[u8] =
        b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";
    let bytes = hash.as_bytes();
    if !(6..=166).contains(&bytes.len()) || !bytes.iter().all(|b| BASE83.contains(b)) {
        return false;
    }
    let size = BASE83.iter().position(|b| *b == bytes[0]).unwrap();
    size <= 80 && bytes.len() == 4 + 2 * (size % 9 + 1) * (size / 9 + 1)
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
        Some(json!({"name":name,"url":url,"id":tag["id"].as_str().filter(|v|https(v)),"image_id":image_id,"width":tag["icon"]["width"].as_u64().filter(|v| *v>0 && *v<=65535),"height":tag["icon"]["height"].as_u64().filter(|v| *v>0 && *v<=65535)}))
    }).collect()
}
pub(super) async fn remember(
    tx: &mut toasty::Transaction<'_>,
    emojis: &[Value],
) -> Result<(), AppError> {
    // Local images use their direct URL, never the remote proxy registry.
    // Validate at this shared sink as well as excluding local feed descriptors.
    let emojis: Vec<_> = emojis
        .iter()
        .filter(|emoji| {
            emoji["local"] != true
                && emoji["image_id"].as_str().is_some_and(|id| {
                    id.len() == 64
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
                && emoji["url"].as_str().is_some_and(https)
        })
        .collect();
    toasty::sql::statement("INSERT INTO apub_emoji_images(id,url) SELECT DISTINCT e->>'image_id',e->>'url' FROM jsonb_array_elements($1::jsonb) e ON CONFLICT(id) DO UPDATE SET last_seen_at=now() WHERE apub_emoji_images.last_seen_at<now()-interval '1 day'").bind(json!(emojis).to_string()).exec(&mut *tx).await?;
    toasty::sql::statement(
        "DELETE FROM apub_emoji_images WHERE last_seen_at<now()-interval '30 days' AND NOT EXISTS(SELECT 1 FROM apub_reactions r WHERE r.emoji->>'image_id'=apub_emoji_images.id)",
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
    json!({"id":value["id"],"type":"Emoji","name":value["name"],"icon":{"type":"Image","url":value["url"],"width":value["width"],"height":value["height"]}})
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
        "SELECT row_to_json(e)::text FROM display_workspace_emojis e ORDER BY name",
        &[],
    )
    .await?;

    rows.into_iter().map(|e| {
        let url = string(&e, "url")?;
        let url = if url.starts_with('/') { format!("{}{url}", base_url().unwrap_or_default()) } else { url.to_owned() };
        let image_id = openssl::sha::sha256(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        Ok(json!({"name":format!(":{}:",string(&e,"name")?),"url":url,"id":url,"image_id":image_id,"local":true,"category":e["category"],"aliases":e["aliases"],"width":e["width"],"height":e["height"]}))
    }).collect()
}

async fn image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> Result<Response, AppError> {
    serve_image(state, id, request, false).await
}
async fn media_image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> Result<Response, AppError> {
    serve_image(state, id, request, true).await
}
async fn serve_image(
    state: AppState,
    id: String,
    request: axum::extract::Request,
    media: bool,
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
    let cached = match emoji_cache::get(
        &format!("{}:{id}", if media { "media" } else { "emoji" }),
        peer,
        || load_image(url, media),
    )
    .await
    {
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
async fn load_image(url: reqwest::Url, media: bool) -> Result<emoji_cache::Image, AppError> {
    let response = transport::safe_client(&url)
        .await?
        .get(url)
        .header(header::ACCEPT, "image/*")
        .send()
        .await
        .map_err(|_| AppError::BadGateway("Emoji image request failed".into()))?;
    read_image(response, media).await
}

pub(super) fn unavailable_image(message: &str) -> AppError {
    AppError::Coded {
        status: axum::http::StatusCode::BAD_GATEWAY,
        code: "unsupported_federated_image",
        message: message.to_owned(),
        extra: Default::default(),
    }
}

async fn read_image(
    mut response: reqwest::Response,
    media: bool,
) -> Result<emoji_cache::Image, AppError> {
    let status = response.status();
    if !status.is_success() {
        return Err(
            if status.is_server_error() || matches!(status.as_u16(), 408 | 429) {
                AppError::BadGateway("Emoji image unavailable".into())
            } else {
                unavailable_image("Emoji image unavailable")
            },
        );
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
        return Err(unavailable_image("Unsupported emoji image"));
    }
    let mut data = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::BadGateway("Emoji image transfer failed".into()))?
    {
        // Emoji retain their 2 MiB limit; photos use 8 MiB. The shared cache is 32 MiB.
        if data.len() + chunk.len() > (if media { 8 } else { 2 }) * 1024 * 1024 {
            return Err(unavailable_image("Emoji image too large"));
        }
        data.extend_from_slice(&chunk);
    }
    let blurhash = if media {
        let kind = mime.clone();
        let permit = crate::routes::home_images::DECODERS
            .acquire()
            .await
            .map_err(|_| AppError::RateLimited)?;
        let (returned_data, hash) = tokio::task::spawn_blocking(move || {
            // Keep the permit in the blocking task if the HTTP request is cancelled.
            let _permit = permit;
            let hash = crate::routes::home_images::image_blurhash(&kind, &data);
            (data, hash)
        })
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        data = returned_data;
        Some(hash.ok_or_else(|| unavailable_image("Invalid image dimensions or pixels"))?)
    } else {
        None
    };
    Ok(emoji_cache::Image {
        mime,
        bytes: data,
        blurhash,
    })
}

// Quote labels use the parsed serialization so Unicode formatting characters cannot
// change the visible destination. Image URL handling intentionally keeps its existing policy.
fn quote_url(value: &str) -> Option<reqwest::Url> {
    if value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'))
    {
        return None;
    }
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.as_str().len() <= 2048)
        .then_some(url)
}

pub(super) fn note_quote_uri(note: &Value) -> Option<reqwest::Url> {
    ["quoteUrl", "quoteUri", "_misskey_quote", "quote"]
        .into_iter()
        .find_map(|key| note[key].as_str().and_then(quote_url))
}

pub(super) fn note_content(note: &Value) -> Result<String, AppError> {
    // Keep the URL in the content for clients that do not render quote cards.
    let quote = note_quote_uri(note);
    let html = match note["content"].as_str() {
        Some(content) => content,
        None if quote.is_some() && note["content"].is_null() => "",
        _ => return Err(AppError::BadRequest("Missing content".into())),
    };
    let source = (note["source"]["mediaType"] == "text/x.misskeymarkdown")
        .then(|| note["source"]["content"].as_str())
        .flatten()
        .or_else(|| note["_misskey_content"].as_str());
    if html.len() > 128 * 1024 || source.is_some_and(|value| value.len() > 128 * 1024) {
        return Err(AppError::BadRequest("Note too large".into()));
    }
    let mut content = source
        .map(str::to_owned)
        .unwrap_or_else(|| plain_content(html));
    if let Some(quote) = quote {
        let quote = quote.as_str();
        let link = format!("<{quote}>");
        let already_linked = content
            .split_whitespace()
            .any(|word| word == quote || word == link);
        if !already_linked {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&link);
        }
    }
    Ok(content)
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

#[cfg(test)]
mod media_proxy_tests {
    use super::*;

    async fn payload(
        bytes: Vec<u8>,
        mime: &str,
        media: bool,
    ) -> Result<emoji_cache::Image, AppError> {
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(bytes)
                    .insert_header("content-type", mime),
            )
            .mount(&upstream)
            .await;
        // Exercise the production response reader; only this test bypasses public HTTPS DNS.
        let response = reqwest::get(upstream.uri()).await.unwrap();
        read_image(response, media).await
    }

    #[tokio::test]
    async fn only_temporary_upstream_failures_are_retryable() {
        let upstream = wiremock::MockServer::start().await;
        for status in [404, 429, 503] {
            wiremock::Mock::given(wiremock::matchers::path(format!("/{status}")))
                .respond_with(wiremock::ResponseTemplate::new(status))
                .mount(&upstream)
                .await;
            let url = format!("{}/{status}", upstream.uri());
            let key = uuid::Uuid::new_v4().to_string();
            let first = emoji_cache::get(&key, None, || async {
                read_image(reqwest::get(&url).await.unwrap(), true).await
            })
            .await;
            let cached = emoji_cache::get(&key, None, || async {
                panic!("cached error was unexpectedly fetched again")
            })
            .await;
            for result in [first, cached] {
                assert_eq!(
                    matches!(result, Err(AppError::BadGateway(_))),
                    status != 404
                );
            }
        }
        assert!(matches!(
            payload(vec![], "video/mp4", true).await,
            Err(AppError::Coded {
                code: "unsupported_federated_image",
                ..
            })
        ));
    }

    fn png(size: usize) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let mut bytes = bytes.into_inner();
        bytes.resize(size.max(bytes.len()), 0);
        bytes
    }

    #[tokio::test]
    async fn received_media_uses_generated_blurhash_only_when_missing() {
        let image = payload(png(0), "image/png", true).await.unwrap();
        let generated = image.blurhash.clone().unwrap();
        assert!(valid_blurhash(&generated));
        let original = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";
        let url = "https://example.test/received-media-blurhash.png";
        let missing = json!({"mediaType":"image/png","url":url});
        let mut media = note_media(&json!({"attachment":[
            missing,
            {"mediaType":"image/png","url":url,"blurhash":original},
            {"mediaType":"video/mp4","url":"https://example.test/movie.mp4"},
        ]}));
        let key = format!("media:{}", media[0]["image_id"].as_str().unwrap());
        emoji_cache::get(&key, None, || async { Ok(image) })
            .await
            .unwrap();

        assert_eq!(
            fill_media_blurhashes(&mut media, None).await,
            (true, Vec::new())
        );

        assert_eq!(media[0]["blurhash"], generated);
        assert_eq!(media[1]["blurhash"], original);
        assert!(media[2]["blurhash"].is_null());
    }

    #[tokio::test]
    async fn blurhash_retry_keeps_only_the_temporarily_failed_image_ids() {
        let mut media = note_media(&json!({"attachment":[
            {"mediaType":"image/png","url":"https://example.test/permanent-blurhash.png"},
            {"mediaType":"image/png","url":"https://example.test/temporary-blurhash.png"},
        ]}));
        let permanent = media[0]["image_id"].as_str().unwrap().to_owned();
        let temporary = media[1]["image_id"].as_str().unwrap().to_owned();
        let _ = emoji_cache::get(&format!("media:{permanent}"), None, || async {
            Err(unavailable_image("Unsupported image"))
        })
        .await;
        let _ = emoji_cache::get(&format!("media:{temporary}"), None, || async {
            Err(AppError::BadGateway("Temporary image failure".into()))
        })
        .await;

        assert_eq!(
            fill_media_blurhashes(&mut media, None).await,
            (false, vec![temporary])
        );
        assert_eq!(
            fill_media_blurhashes(&mut media, Some(&[json!(permanent)])).await,
            (false, Vec::new())
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL with ActivityPub tables"]
    async fn delayed_blurhash_write_keeps_a_newer_attachment_version() -> anyhow::Result<()> {
        let mut db = crate::db::init_db(&std::env::var("DATABASE_URL")?).await?;
        let actor = format!("https://example.test/users/{}", uuid::Uuid::new_v4());
        let note = format!("https://example.test/notes/{}", uuid::Uuid::new_v4());
        let old: jiff::Timestamp = "2026-01-01T00:00:00Z".parse()?;
        let new: jiff::Timestamp = "2026-01-02T00:00:00Z".parse()?;
        let original = r#"[{"url":"https://example.test/photo.png"}]"#;
        let replacement = r#"[{"url":"https://example.test/replacement.png"}]"#;
        let enriched = r#"[{"url":"https://example.test/photo.png","blurhash":"000000"}]"#;
        toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'peer','Peer',$1,'test')")
            .bind(actor.as_str()).exec(&mut db).await?;
        toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,media) VALUES($1,$2,'original','public',$3::timestamptz,$3::timestamptz,$4::jsonb)")
            .bind(note.as_str()).bind(actor.as_str()).bind(old.to_string()).bind(original).exec(&mut db).await?;

        // Exercise the two ways a newer delivery can replace the saved version.
        toasty::sql::statement(
            "UPDATE apub_remote_posts SET content='updated',updated_at=$2::timestamptz WHERE id=$1",
        )
        .bind(note.as_str())
        .bind(new.to_string())
        .exec(&mut db)
        .await?;
        store_blurhashes(&mut db, &note, &actor, old, original, enriched).await?;
        let mut tx = db.transaction().await?;
        let row = one(
            &mut tx,
            "SELECT json_build_object('media',media)::text FROM apub_remote_posts WHERE id=$1",
            &[&note],
        )
        .await?;
        assert_eq!(row["media"], serde_json::from_str::<Value>(original)?);
        tx.commit().await?;

        toasty::sql::statement("UPDATE apub_remote_posts SET media=$2::jsonb WHERE id=$1")
            .bind(note.as_str())
            .bind(replacement)
            .exec(&mut db)
            .await?;
        store_blurhashes(&mut db, &note, &actor, new, original, enriched).await?;

        let mut tx = db.transaction().await?;
        let row = one(&mut tx, "SELECT json_build_object('content',content,'media',media)::text FROM apub_remote_posts WHERE id=$1", &[&note]).await?;
        assert_eq!(row["content"], "updated");
        assert_eq!(row["media"], serde_json::from_str::<Value>(replacement)?);
        Ok(())
    }

    #[tokio::test]
    async fn photo_and_emoji_transfer_limits_and_raster_validation() {
        assert!(
            payload(png(2 * 1024 * 1024), "image/png", false)
                .await
                .is_ok()
        );
        assert!(
            payload(png(2 * 1024 * 1024 + 1), "image/png", false)
                .await
                .is_err()
        );
        assert!(
            payload(png(8 * 1024 * 1024), "image/png", true)
                .await
                .is_ok()
        );
        assert!(
            payload(png(8 * 1024 * 1024 + 1), "image/png", true)
                .await
                .is_err()
        );
        assert!(
            payload(b"not a PNG".to_vec(), "image/png", true)
                .await
                .is_err()
        );
        assert!(payload(png(0), "image/jpeg", true).await.is_err());
        assert!(
            payload(b"<svg/>".to_vec(), "image/svg+xml", true)
                .await
                .is_err()
        );
    }
}

#[cfg(test)]
mod media_metadata_tests {
    use super::*;

    #[test]
    fn preserves_passive_media_without_registering_them_as_images() {
        for mime in [
            "application/pdf",
            "video/mp4",
            "audio/mpeg",
            "message/rfc822",
            "text/plain",
            "application/zip",
            "application/x-zip-compressed",
        ] {
            let received = media(&json!({"url":"https://peer.example/file", "mediaType":mime}));
            assert_eq!(received.len(), 1);
            assert_eq!(received[0]["media_type"], mime);
            assert!(received[0].get("image_id").is_none());
        }
        for mime in [
            "image/svg+xml",
            "application/javascript",
            "application/x-msdownload",
        ] {
            assert!(
                media(&json!({"url":"https://peer.example/file", "mediaType":mime})).is_empty()
            );
        }
        assert!(
            media(&json!({"url":"http://peer.example/file", "mediaType":"video/mp4"})).is_empty()
        );
    }

    #[test]
    fn preserves_federated_metadata_and_ignores_invalid_optional_values() {
        let hash = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";
        let mut attachment = json!({"type":"Document","url":"https://peer.example/photo.png",
            "mediaType":"image/png","width":640,"height":480,"blurhash":hash});
        let received = media(&attachment);
        assert_eq!(received[0]["width"], 640);
        assert_eq!(received[0]["height"], 480);
        assert_eq!(received[0]["blurhash"], hash);
        for invalid in [
            json!("bad"),
            json!("0000000"),
            json!("00000!"),
            json!("~00000"),
            json!("é00000"),
            json!(42),
        ] {
            attachment["blurhash"] = invalid;
            attachment["width"] = json!(-1);
            attachment["height"] = json!(2147483648_u64);
            let received = media(&attachment);
            assert_eq!(received.len(), 1);
            for key in ["blurhash", "width", "height"] {
                assert!(received[0].get(key).is_none());
            }
        }
        for key in ["blurhash", "width", "height"] {
            attachment.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(media(&attachment).len(), 1);
        let wrapped = json!([attachment]);
        assert_eq!(media(&wrapped).len(), 1);
    }
}
