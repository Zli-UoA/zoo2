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
    stream::iter(values)
        .for_each_concurrent(8, |emoji| async move {
            if !matches!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    backfill_one(state, &emoji)
                )
                .await,
                Ok(Ok(()))
            ) {
                tracing::warn!(name=?emoji["name"], "Could not backfill emoji dimensions");
            }
        })
        .await;
    Ok(())
}

async fn backfill_one(state: &AppState, emoji: &serde_json::Value) -> anyhow::Result<()> {
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
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE workspace_emojis SET width=$2,height=$3,image_id=COALESCE(NULLIF($4,''),image_id) WHERE url=$1")
        .bind(url).bind(width).bind(height).bind(emoji_image_id(url).unwrap_or_default()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(())
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
