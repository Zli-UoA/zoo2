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

/// 転送元メッセージ ID 配列を UUID として検証・重複排除する。
/// 大文字・別表記で作成された既存 `messages.id` も主キー検索できるよう、元の表記を保持する。
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
        let trimmed = raw.trim();
        let uuid = Uuid::parse_str(trimmed).map_err(|_| {
            AppError::BadRequest(format!("forwarded_message_id is not a valid UUID: {raw}"))
        })?;
        if seen.insert(uuid) {
            normalized.push(trimmed.to_string());
        }
    }
    Ok(normalized)
}

fn uuid_lookup_candidates(raw: &str) -> Result<(Uuid, Vec<String>), AppError> {
    let trimmed = raw.trim();
    let uuid = Uuid::parse_str(trimmed).map_err(|_| {
        AppError::BadRequest(format!("forwarded_message_id is not a valid UUID: {raw}"))
    })?;
    let lower_hyphen = uuid.hyphenated().to_string();
    let upper_hyphen = lower_hyphen.to_uppercase();
    let lower_simple = uuid.simple().to_string();
    let upper_simple = lower_simple.to_uppercase();
    let lower_braced = uuid.braced().to_string();
    let upper_braced = lower_braced.to_uppercase();
    let mut candidates = Vec::with_capacity(7);
    for candidate in [
        trimmed.to_string(),
        lower_hyphen,
        upper_hyphen,
        lower_simple,
        upper_simple,
        lower_braced,
        upper_braced,
    ] {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    Ok((uuid, candidates))
}

struct ExpandedLookupCandidates {
    reqs: Vec<(String, Uuid)>,
    candidates: Vec<String>,
}

fn expand_lookup_candidates(
    ids: &[String],
    exact_present: &HashSet<String>,
) -> Result<ExpandedLookupCandidates, AppError> {
    let mut reqs = Vec::with_capacity(ids.len());
    let mut candidates = Vec::with_capacity(ids.len() * 2);
    let mut seen = HashSet::with_capacity(ids.len() * 4);
    for req_id in ids {
        let (uuid, id_candidates) = uuid_lookup_candidates(req_id)?;
        reqs.push((req_id.clone(), uuid));
        if exact_present.contains(req_id) {
            // 要求文字列と完全一致する行が DB 上に存在する場合は、他表記の別行へフォールバックさせない
            if seen.insert(req_id.clone()) {
                candidates.push(req_id.clone());
            }
        } else {
            for candidate in id_candidates {
                if seen.insert(candidate.clone()) {
                    candidates.push(candidate);
                }
            }
        }
    }
    Ok(ExpandedLookupCandidates { reqs, candidates })
}

async fn load_exact_present_ids(
    tx: &mut toasty::Transaction<'_>,
    ids: &[String],
) -> Result<HashSet<String>, AppError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let exact_json = Value::from(ids.to_vec()).to_string();
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('id', m.id)::text \
         FROM jsonb_array_elements_text($1::jsonb) AS req(id) \
         JOIN messages m ON m.id = req.id",
        &[exact_json.as_str()],
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect())
}

/// 送信時権限検証 SQL（スケール検証テストと共有）。
///
/// `$1` = UUID 候補文字列配列 JSON、`$2` = 送信者 `users.id`、`$3` = bot か、`$4` = guest か。
pub const VERIFY_FORWARD_SOURCES_SQL: &str = "\
SELECT json_build_object('id', m.id)::text
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

/// 送信者自身が、指定した全転送元メッセージを閲覧可能（未削除・平文チャンネル/DM・権限あり）であることを検証し、
/// データベースに保存されている実際の `messages.id` 表記を返す。
/// 存在しない・削除済み・閲覧権限なし・暗号化チャンネルのいずれであっても 404 を返し、存在オラクルを防ぐ。
pub async fn verify_forward_sources(
    tx: &mut toasty::Transaction<'_>,
    actor_id: &str,
    bot_actor: bool,
    guest_actor: bool,
    ids: &[String],
) -> Result<Vec<String>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let exact_present = load_exact_present_ids(tx, ids).await?;
    let ExpandedLookupCandidates {
        reqs: parsed_reqs,
        candidates,
    } = expand_lookup_candidates(ids, &exact_present)?;
    let ids_json = Value::from(candidates).to_string();
    let rows = crate::apps::query(
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
    let mut matched_exact: HashSet<String> = HashSet::with_capacity(rows.len());
    let mut matched_by_uuid: HashMap<Uuid, Vec<String>> = HashMap::with_capacity(rows.len());
    for row in rows {
        if let Some(id) = row.get("id").and_then(|v| v.as_str())
            && let Ok(uuid) = Uuid::parse_str(id)
        {
            matched_exact.insert(id.to_string());
            matched_by_uuid
                .entry(uuid)
                .or_default()
                .push(id.to_string());
        }
    }
    let mut resolved = Vec::with_capacity(parsed_reqs.len());
    let mut seen = HashSet::with_capacity(parsed_reqs.len());
    for (req_id, uuid) in parsed_reqs {
        let actual_id = if exact_present.contains(&req_id) {
            if matched_exact.contains(&req_id) {
                req_id
            } else {
                return Err(AppError::NotFound("Forwarded message not found".into()));
            }
        } else {
            let Some(matches) = matched_by_uuid.get(&uuid) else {
                return Err(AppError::NotFound("Forwarded message not found".into()));
            };
            if matches.len() != 1 {
                return Err(AppError::NotFound("Forwarded message not found".into()));
            }
            matches[0].clone()
        };
        if seen.insert(actual_id.clone()) {
            resolved.push(actual_id);
        }
    }
    Ok(resolved)
}

/// 転送元メッセージの一括解決 SQL（スケール検証テストと共有）。
///
/// `$1` = 重複排除済みの UUID 配列 JSON（最大 100 件）、`$2` = 閲覧者 `users.id`。
/// 入力 ID 集合から主キー・索引で `messages`・`channels`・`users`・`channel_user`・`slack_forward_targets`・`slack_channels` を参照し、
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
LEFT JOIN slack_forward_targets sft ON sft.message_id = m.id
LEFT JOIN slack_channels sft_sc
  ON sft.message_id IS NOT NULL
  AND sft_sc.team_id = sft.team_id
  AND sft_sc.slack_channel_id = sft.slack_channel_id
WHERE NOT viewer.is_deactivated
  AND c.deleted_at IS NULL
  AND NOT c.encrypted
  AND (
    (NOT c.is_private AND NOT c.is_dm AND NOT viewer.is_guest AND NOT viewer.is_bot)
    OR cu.user_id IS NOT NULL
  )
  AND (
    CASE
      WHEN sft.message_id IS NOT NULL THEN
        m.created_at = (to_timestamp(sft.ts::numeric) AT TIME ZONE 'UTC')::timestamp(3)
        AND (sft_sc.channel_id IS NULL OR m.channel_id = sft_sc.channel_id)
      ELSE true
    END
  )";

async fn resolve_forwards(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<ResolveForwardsRequest>,
) -> Result<Json<ApiResponse<ResolveForwardsResponse>>, AppError> {
    if payload.ids.len() > MAX_RESOLVE_FORWARD_IDS {
        return Err(AppError::BadRequest(format!(
            "Too many forwarded messages: maximum allowed is {MAX_RESOLVE_FORWARD_IDS}, got {}",
            payload.ids.len()
        )));
    }
    // 解決 API では同一 UUID の別表記（小文字／大文字等）が同じ要求に含まれる場合も各キーを返す。
    let mut ids = Vec::with_capacity(payload.ids.len());
    let mut seen_keys = HashSet::with_capacity(payload.ids.len());
    for raw in &payload.ids {
        let trimmed = raw.trim();
        if Uuid::parse_str(trimmed).is_err() {
            return Err(AppError::BadRequest(format!(
                "forwarded_message_id is not a valid UUID: {raw}"
            )));
        }
        if seen_keys.insert(trimmed.to_string()) {
            ids.push(trimmed.to_string());
        }
    }
    if ids.is_empty() {
        return Ok(Json(ApiResponse::new(
            ResolveForwardsResponse {
                items: HashMap::new(),
            },
            None,
        )));
    }

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let exact_present = load_exact_present_ids(&mut tx, &ids).await?;
    let ExpandedLookupCandidates {
        reqs: parsed_reqs,
        candidates,
    } = expand_lookup_candidates(&ids, &exact_present)?;
    let ids_json = Value::from(candidates).to_string();
    let rows = crate::apps::query(
        &mut tx,
        RESOLVE_FORWARDS_SQL,
        &[ids_json.as_str(), auth_user.id.as_str()],
    )
    .await?;
    tx.commit().await?;

    let mut exact_items: HashMap<String, ForwardResolutionItem> =
        HashMap::with_capacity(rows.len());
    let mut items_by_uuid: HashMap<Uuid, Vec<ForwardResolutionItem>> =
        HashMap::with_capacity(rows.len());
    for mut row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let parsed_uuid = Uuid::parse_str(&id).ok();
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unavailable");
        let parsed_item = if status == "deleted" {
            Some(ForwardResolutionItem::Deleted)
        } else if status == "ok" {
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
            serde_json::from_value::<ResolvedForwardMessage>(row)
                .ok()
                .map(|resolved| ForwardResolutionItem::Ok(Box::new(resolved)))
        } else {
            None
        };
        if let Some(item) = parsed_item {
            if let Some(u) = parsed_uuid {
                items_by_uuid.entry(u).or_default().push(item.clone());
            }
            exact_items.insert(id, item);
        }
    }

    let mut items: HashMap<String, ForwardResolutionItem> =
        HashMap::with_capacity(parsed_reqs.len());
    for (req_id, req_uuid) in parsed_reqs {
        let item = if exact_present.contains(&req_id) {
            exact_items
                .get(&req_id)
                .cloned()
                .unwrap_or(ForwardResolutionItem::Unavailable)
        } else {
            match items_by_uuid.get(&req_uuid) {
                Some(matches) if matches.len() == 1 => matches[0].clone(),
                _ => ForwardResolutionItem::Unavailable,
            }
        };
        items.insert(req_id, item);
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
        let a = " 550E8400-E29B-41D4-A716-446655440000 ".to_string();
        let b = "550e8400-e29b-41d4-a716-446655440000".to_string();
        let c = "6ba7b810-9dad-11d1-80b4-00c04fd430c8".to_string();
        let out = normalize_forwarded_message_ids(&[a, b, c], 5).unwrap();
        assert_eq!(
            out,
            vec![
                "550E8400-E29B-41D4-A716-446655440000",
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
