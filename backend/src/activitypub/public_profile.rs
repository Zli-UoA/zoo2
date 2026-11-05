//! Anonymous profile data; presentation belongs to the shared frontend components.
use super::*;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
};
use serde::Deserialize;

// Only explicit HTML requests opt in. Missing Accept and */* keep the federation
// representation; ties prefer ActivityPub when a client requests both formats.
pub(super) fn prefers_html(headers: &HeaderMap) -> bool {
    let mut html = 0.0_f32;
    let mut json = 0.0_f32;
    for value in headers.get_all(header::ACCEPT) {
        let Ok(value) = value.to_str() else { continue };
        for range in value.split(',') {
            let mut parts = range.split(';');
            let media = parts.next().unwrap_or_default().trim();
            let quality = parts
                .filter_map(|part| part.trim().split_once('='))
                .find(|(name, _)| name.eq_ignore_ascii_case("q"))
                .map(|(_, value)| value.trim().parse::<f32>().unwrap_or(0.0))
                .unwrap_or(1.0);
            if !(0.0..=1.0).contains(&quality) {
                continue;
            }
            if media.eq_ignore_ascii_case("text/html") {
                html = html.max(quality);
            } else if [
                "application/activity+json",
                "application/ld+json",
                "application/json",
            ]
            .iter()
            .any(|candidate| media.eq_ignore_ascii_case(candidate))
            {
                json = json.max(quality);
            }
        }
    }
    html > json
}

pub(super) fn page_url(base: &str, id: &str) -> String {
    format!(
        "{base}/public/users/{}",
        percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC)
    )
}

#[derive(Default, Deserialize)]
pub(super) struct ProfileQuery {
    before: Option<String>,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(page): Query<ProfileQuery>,
) -> Result<Json<Value>, AppError> {
    let base = base_url()?;
    let mut account = routes::local_user(&state, &id).await?;
    account["actor_id"] = json!(actor_id(&id)?);
    account["url"] = json!(page_url(&base, &id));
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let mut posts = query(&mut tx, "SELECT json_build_object('id',id,'content',content,'created_at',created_at,'media',media)::text FROM home_posts p WHERE user_id=$1 AND apub AND visibility='public' AND deleted_at IS NULL AND (p.channel_id IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=p.channel_id AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)) AND ($2='' OR (created_at,id)<(SELECT created_at,id FROM home_posts p WHERE id=$2 AND user_id=$1 AND apub AND visibility='public' AND deleted_at IS NULL AND (p.channel_id IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=p.channel_id AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)))) ORDER BY created_at DESC,id DESC LIMIT 21", &[&id, page.before.as_deref().unwrap_or("")]).await?;
    crate::routes::home_images::decorate(&mut tx, &mut posts).await?;
    let next_cursor = if posts.len() > 20 {
        Some(string(&posts[19], "id")?.to_owned())
    } else {
        None
    };
    posts.truncate(20);
    for post in &mut posts {
        *post = super::with_workspace_emojis(&mut tx, post).await?;
    }
    Ok(Json(
        json!({"account":account,"posts":posts,"next_cursor":next_cursor}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_and_federation_accept_negotiation() {
        for (accept, expected) in [
            (
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                true,
            ),
            ("*/*", false),
            ("application/activity+json", false),
            (
                "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
                false,
            ),
            ("text/html,application/activity+json", false),
            ("text/html;q=0.9,application/activity+json;q=0.5", true),
            ("text/html;q=0", false),
            ("text/html;q=bad", false),
            ("text/html;q=NaN", false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT, accept.parse().unwrap());
            assert_eq!(prefers_html(&headers), expected, "{accept}");
        }
        assert!(!prefers_html(&HeaderMap::new()));
        let mut headers = HeaderMap::new();
        headers.append(header::ACCEPT, "text/html".parse().unwrap());
        headers.append(header::ACCEPT, "application/activity+json".parse().unwrap());
        assert!(!prefers_html(&headers));
    }
}
