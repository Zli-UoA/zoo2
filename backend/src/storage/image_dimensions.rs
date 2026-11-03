//! Read image headers without decoding full images. Dimensions travel with the
//! display read model so animated and wide emojis reserve the correct space.
use crate::{error::AppError, state::AppState};

pub fn emoji_image_id(url: &str) -> Option<String> {
    let base = crate::activitypub::base_url().ok()?;
    let url = if url.starts_with('/') {
        format!("{}{url}", base.trim_end_matches('/'))
    } else {
        url.to_owned()
    };
    Some(
        openssl::sha::sha256(url.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

pub fn dimensions(bytes: &[u8]) -> Option<(i32, i32)> {
    let size = imagesize::blob_size(bytes).ok()?;
    let width = i32::try_from(size.width).ok()?;
    let height = i32::try_from(size.height).ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// Upgrade pre-existing uploads before accepting traffic. Failed/missing objects
/// keep a stable square fallback and can be retried at the next startup.
pub async fn backfill_emojis(state: &AppState) -> Result<(), AppError> {
    use futures_util::{StreamExt, stream};
    let values = {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        let sql = if crate::activitypub::base_url().is_ok() {
            "SELECT row_to_json(e)::text FROM workspace_emojis e WHERE width IS NULL OR height IS NULL OR image_id IS NULL"
        } else {
            "SELECT row_to_json(e)::text FROM workspace_emojis e WHERE width IS NULL OR height IS NULL"
        };
        crate::apps::query(&mut tx, sql, &[]).await?
    };
    // Leave time inside the startup's 30s budget to commit successful reads.
    // Otherwise a large/slow registry would lose the whole batch on every boot.
    let read_budget = tokio::time::sleep(std::time::Duration::from_secs(20));
    // Fetch headers first; publish one statement so the transition-table
    // trigger runs once, with only the actually changed emoji names.
    let updates = stream::iter(values)
        .map(|emoji| async move {
            match tokio::time::timeout(std::time::Duration::from_secs(10), read_one(state, &emoji))
                .await
            {
                Ok(Ok(value)) => Some(value),
                _ => {
                    tracing::warn!(name=?emoji["name"], "Could not backfill emoji dimensions");
                    None
                }
            }
        })
        .buffer_unordered(8)
        .filter_map(|value| async move { value })
        .take_until(read_budget)
        .collect::<Vec<_>>()
        .await;
    if !updates.is_empty() {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("UPDATE workspace_emojis e SET width=v.width,height=v.height,image_id=COALESCE(NULLIF(v.image_id,''),e.image_id) FROM jsonb_to_recordset($1::jsonb) AS v(url text,width integer,height integer,image_id text) WHERE e.url=v.url AND ROW(e.width,e.height,e.image_id) IS DISTINCT FROM ROW(v.width,v.height,COALESCE(NULLIF(v.image_id,''),e.image_id))")
            .bind(serde_json::to_string(&updates).expect("serializable emoji metadata"))
            .exec(&mut tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn read_one(
    state: &AppState,
    emoji: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let url = emoji["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing emoji URL"))?;
    let id = url
        .strip_prefix("/api/emoji-images/")
        .ok_or_else(|| anyhow::anyhow!("Not a managed emoji"))?;
    uuid::Uuid::parse_str(id)?;
    let existing = emoji["width"].as_i64().zip(emoji["height"].as_i64());
    let (width, height) = if let Some((width, height)) = existing {
        (i32::try_from(width)?, i32::try_from(height)?)
    } else {
        let object = state
            .s3_client
            .get_object()
            .bucket(&state.config.s3.bucket)
            .key(format!("custom-emojis/{id}"))
            .send()
            .await?;
        if object
            .content_length()
            .is_none_or(|size| !(1..=2 * 1024 * 1024).contains(&size))
        {
            anyhow::bail!("Invalid emoji object size");
        }
        let bytes = object.body.collect().await?.into_bytes();
        dimensions(&bytes).ok_or_else(|| anyhow::anyhow!("Invalid emoji image"))?
    };
    Ok(
        serde_json::json!({"url":url,"width":width,"height":height,"image_id":emoji_image_id(url).unwrap_or_default()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wide_gif_headers_without_decoding_frames() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[120, 0, 20, 0, 0, 0, 0]);
        assert_eq!(dimensions(&gif), Some((120, 20)));
        assert_eq!(dimensions(b"invalid"), None);
        assert_eq!(dimensions(b"GIF89a\0\0\0\0\0\0\0"), None);
    }
}
