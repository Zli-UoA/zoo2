use std::collections::{HashMap, HashSet};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, header},
    middleware,
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{auth::AuthUser, error::AppError, routes::response::ApiResponse, state::AppState};

/// 1 メッセージが保持できる転送先 ID の上限（Slack の複数メッセージ展開を含む）。
pub const MAX_FORWARDED_MESSAGES_PER_MESSAGE: usize = 5;
/// ユーザーのメッセージ送信 API（`POST /api/messages`）で一度に指定できる転送元の上限。
pub const MAX_FORWARDED_MESSAGES_USER_POST: usize = 1;
/// 一括解決 API（`POST /api/messages/forwards/resolve`）で一度に受け付ける ID の上限。
pub const MAX_RESOLVE_FORWARD_IDS: usize = 100;

#[derive(Debug, Deserialize)]
pub struct ResolveForwardsRequest {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmParticipant {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardAuthor {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedForwardMessage {
    pub id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub is_private: bool,
    pub is_dm: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dm_participants: Vec<DmParticipant>,
    pub parent_id: Option<String>,
    pub user_id: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub content: String,
    pub card: Option<String>,
    pub components: Option<String>,
    pub message_kind: Option<String>,
    pub author: Option<ForwardAuthor>,
    pub emojis: Value,
    pub attachments: Vec<Value>,
    pub has_forwards: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ForwardResolutionItem {
    Ok(Box<ResolvedForwardMessage>),
    Deleted,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveForwardsResponse {
    pub items: HashMap<String, ForwardResolutionItem>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/messages/forwards/resolve", post(resolve_forwards))
        .layer(middleware::map_response(
            |mut response: axum::response::Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

/// 転送元メッセージ ID 配列を検証し、小文字 UUID に正規化・重複排除する。
pub fn normalize_forwarded_message_ids(
    ids: &[String],
    max_len: usize,
) -> Result<Vec<String>, AppError> {
    if ids.len() > max_len {
        return Err(AppError::BadRequest(format!(
            "Too many forwarded messages: maximum allowed is {max_len}, got {}",
            ids.len()
        )));
    }
    let mut normalized = Vec::with_capacity(ids.len());
    let mut seen = HashSet::with_capacity(ids.len());
    for raw in ids {
        let uuid = Uuid::parse_str(raw.trim()).map_err(|_| {
            AppError::BadRequest(format!("forwarded_message_id is not a valid UUID: {raw}"))
        })?;
        let canonical = uuid.to_string();
        if seen.insert(canonical.clone()) {
            normalized.push(canonical);
        }
    }
    Ok(normalized)
}

/// 送信時権限検証 SQL（スケール検証テストと共有）。
///
/// `$1` = 重複排除済みの UUID 配列 JSON、`$2` = 送信者 `users.id`、`$3` = bot か、`$4` = guest か。
pub const VERIFY_FORWARD_SOURCES_SQL: &str = "\
SELECT json_build_object('count', count(*))::text
FROM jsonb_array_elements_text($1::jsonb) AS req(id)
JOIN messages m ON m.id = req.id
JOIN channels c ON c.id = m.channel_id
LEFT JOIN channel_user cu ON cu.user_id = $2 AND cu.channel_id = c.id
WHERE m.deleted_at IS NULL
  AND c.deleted_at IS NULL
  AND NOT c.encrypted
  AND (
    (NOT c.is_private AND NOT c.is_dm AND NOT $3::boolean AND NOT $4::boolean)
    OR cu.user_id IS NOT NULL
  )";

/// 送信者自身が、指定した全転送元メッセージを閲覧可能（未削除・平文チャンネル/DM・権限あり）であることを検証する。
/// 存在しない・削除済み・閲覧権限なし・暗号化チャンネルのいずれであっても 404 を返し、存在オラクルを防ぐ。
pub async fn verify_forward_sources(
    tx: &mut toasty::Transaction<'_>,
    actor_id: &str,
    bot_actor: bool,
    guest_actor: bool,
    ids: &[String],
) -> Result<(), AppError> {
    if ids.is_empty() {
        return Ok(());
    }
    let ids_json = Value::from(ids.to_vec()).to_string();
    let row = crate::apps::one(
        tx,
        VERIFY_FORWARD_SOURCES_SQL,
        &[
            ids_json.as_str(),
            actor_id,
            if bot_actor { "true" } else { "false" },
            if guest_actor { "true" } else { "false" },
        ],
    )
    .await?;
    let count = row["count"].as_i64().unwrap_or(0);
    if count != ids.len() as i64 {
        return Err(AppError::NotFound("Forwarded message not found".into()));
    }
    Ok(())
}

/// 転送元メッセージの一括解決 SQL（スケール検証テストと共有）。
///
/// `$1` = 重複排除済みの UUID 配列 JSON（最大 100 件）、`$2` = 閲覧者 `users.id`。
/// 入力 ID 集合から主キーで `messages`・`channels`・`users`・`channel_user` のみを参照し、
/// 総履歴 N に依存せず O(K log N) で解決する。
pub const RESOLVE_FORWARDS_SQL: &str = "\
SELECT json_build_object(
  'id', m.id,
  'status', CASE WHEN m.deleted_at IS NOT NULL THEN 'deleted' ELSE 'ok' END,
  'channel_id', CASE WHEN m.deleted_at IS NULL THEN c.id ELSE NULL END,
  'channel_name', CASE WHEN m.deleted_at IS NULL THEN c.channel_name ELSE NULL END,
  'is_private', CASE WHEN m.deleted_at IS NULL THEN c.is_private ELSE NULL END,
  'is_dm', CASE WHEN m.deleted_at IS NULL THEN c.is_dm ELSE NULL END,
  'dm_participants', CASE WHEN m.deleted_at IS NULL AND c.is_dm THEN COALESCE((
    SELECT jsonb_agg(
      jsonb_build_object('id', pu.id, 'user_name', pu.user_name, 'display_name', pu.display_name)
      ORDER BY pu.id
    )
    FROM (
      SELECT cu2.user_id
      FROM channel_user cu2
      WHERE cu2.channel_id = c.id
      ORDER BY cu2.user_id
      LIMIT 10
    ) p
    JOIN users pu ON pu.id = p.user_id
  ), '[]'::jsonb) ELSE NULL END,
  'parent_id', CASE WHEN m.deleted_at IS NULL THEN m.parent_id ELSE NULL END,
  'user_id', CASE WHEN m.deleted_at IS NULL THEN m.user_id ELSE NULL END,
  'created_at', CASE WHEN m.deleted_at IS NULL THEN m.created_at::text ELSE NULL END,
  'updated_at', CASE WHEN m.deleted_at IS NULL AND m.updated_at IS NOT NULL THEN m.updated_at::text ELSE NULL END,
  'content', CASE WHEN m.deleted_at IS NULL THEN m.content ELSE NULL END,
  'card', CASE WHEN m.deleted_at IS NULL THEN m.card ELSE NULL END,
  'components', CASE WHEN m.deleted_at IS NULL THEN m.components ELSE NULL END,
  'message_kind', CASE WHEN m.deleted_at IS NULL THEN m.message_kind ELSE NULL END,
  'author', CASE WHEN m.deleted_at IS NULL THEN COALESCE(
    NULLIF(m.display_data->'author', 'null'::jsonb),
    (SELECT jsonb_build_object(
      'id', au.id,
      'user_name', au.user_name,
      'display_name', au.display_name,
      'avatar_url', au.avatar_url,
      'is_bot', au.is_bot,
      'app_id', au.app_id
    ) FROM users au WHERE au.id = m.user_id)
  ) ELSE NULL END,
  'emojis', CASE WHEN m.deleted_at IS NULL THEN COALESCE(m.display_data->'emojis', '{}'::jsonb) ELSE NULL END,
  'attachments', CASE WHEN m.deleted_at IS NULL THEN COALESCE(
    NULLIF(m.display_data->'attachments', 'null'::jsonb),
    (SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.created_at, a.id), '[]'::jsonb)
     FROM attachments a WHERE a.message_id = m.id)
  ) ELSE NULL END,
  'has_forwards', CASE WHEN m.deleted_at IS NULL THEN (
    m.forwarded_message_ids IS NOT NULL
    AND jsonb_typeof(m.forwarded_message_ids) = 'array'
    AND jsonb_array_length(m.forwarded_message_ids) > 0
  ) ELSE NULL END
)::text
FROM jsonb_array_elements_text($1::jsonb) AS req(id)
JOIN users viewer ON viewer.id = $2
JOIN messages m ON m.id = req.id
JOIN channels c ON c.id = m.channel_id
LEFT JOIN channel_user cu ON cu.user_id = $2 AND cu.channel_id = c.id
WHERE NOT viewer.is_deactivated
  AND c.deleted_at IS NULL
  AND NOT c.encrypted
  AND (
    (NOT c.is_private AND NOT c.is_dm AND NOT viewer.is_guest AND NOT viewer.is_bot)
    OR cu.user_id IS NOT NULL
  )";

async fn resolve_forwards(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<ResolveForwardsRequest>,
) -> Result<Json<ApiResponse<ResolveForwardsResponse>>, AppError> {
    let ids = normalize_forwarded_message_ids(&payload.ids, MAX_RESOLVE_FORWARD_IDS)?;
    if ids.is_empty() {
        return Ok(Json(ApiResponse::new(
            ResolveForwardsResponse {
                items: HashMap::new(),
            },
            None,
        )));
    }

    let ids_json = Value::from(ids.clone()).to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = crate::apps::query(
        &mut tx,
        RESOLVE_FORWARDS_SQL,
        &[ids_json.as_str(), auth_user.id.as_str()],
    )
    .await?;
    tx.commit().await?;

    let mut items: HashMap<String, ForwardResolutionItem> = HashMap::with_capacity(ids.len());
    for mut row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unavailable");
        if status == "deleted" {
            items.insert(id, ForwardResolutionItem::Deleted);
            continue;
        }
        if status == "ok" {
            // `CASE ... ELSE NULL` で JSON 上に null が入る任意配列フィールドを補完してからデシリアライズする。
            if let Some(obj) = row.as_object_mut() {
                if obj.get("dm_participants").is_none_or(Value::is_null) {
                    obj.insert("dm_participants".into(), Value::Array(Vec::new()));
                }
                if obj.get("attachments").is_none_or(Value::is_null) {
                    obj.insert("attachments".into(), Value::Array(Vec::new()));
                }
                if obj.get("emojis").is_none_or(Value::is_null) {
                    obj.insert("emojis".into(), Value::Object(serde_json::Map::new()));
                }
                if obj.get("has_forwards").is_none_or(Value::is_null) {
                    obj.insert("has_forwards".into(), Value::Bool(false));
                }
            }
            if let Ok(resolved) = serde_json::from_value::<ResolvedForwardMessage>(row) {
                items.insert(id, ForwardResolutionItem::Ok(Box::new(resolved)));
            }
        }
    }
    for id in ids {
        items
            .entry(id)
            .or_insert(ForwardResolutionItem::Unavailable);
    }

    Ok(Json(ApiResponse::new(
        ResolveForwardsResponse { items },
        None,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_deduplicates_forwarded_ids() {
        let a = "550E8400-E29B-41D4-A716-446655440000".to_string();
        let b = "550e8400-e29b-41d4-a716-446655440000".to_string();
        let c = "6ba7b810-9dad-11d1-80b4-00c04fd430c8".to_string();
        let out = normalize_forwarded_message_ids(&[a, b, c], 5).unwrap();
        assert_eq!(
            out,
            vec![
                "550e8400-e29b-41d4-a716-446655440000",
                "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            ]
        );
    }

    #[test]
    fn rejects_invalid_uuid_or_exceeding_max_len() {
        let err = normalize_forwarded_message_ids(&["not-a-uuid".into()], 5).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        let id1 = Uuid::new_v4().to_string();
        let id2 = Uuid::new_v4().to_string();
        let err = normalize_forwarded_message_ids(&[id1, id2], 1).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
