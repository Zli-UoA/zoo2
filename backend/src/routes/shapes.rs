use std::collections::HashMap;

use axum::{
    Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use uuid::Uuid;

use crate::{
    auth::tenant::TenantAuth,
    error::AppError,
    models::{Channel, ChannelUser},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/shapes/channels", get(channels_shape))
        .route("/api/shapes/messages", get(messages_shape))
        .route(
            "/api/shapes/message_reactions",
            get(message_reactions_shape),
        )
        .route("/api/shapes/users", get(users_shape))
        .route("/api/shapes/channel_users", get(channel_users_shape))
}

/// クライアントから送信されたクエリパラメータのうち、Electric プロトコルで許可されたものを転送
pub(crate) fn filter_electric_params(raw_query: &HashMap<String, String>) -> Vec<(String, String)> {
    let allowed_prefixes = ["subset__"];
    let allowed_keys = [
        "live",
        "live_sse",
        "experimental_live_sse",
        "handle",
        "offset",
        "cursor",
        "expired_handle",
        "log",
        "replica",
        "cache-buster",
    ];

    raw_query
        .iter()
        .filter(|(k, _)| {
            allowed_keys.contains(&k.as_str())
                || allowed_prefixes.iter().any(|prefix| k.starts_with(prefix))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Channels Shape の where 句とパラメータの純粋組み立て
pub(crate) fn channels_shape_where(user_id: &str) -> (String, Vec<(&'static str, &str)>) {
    (
        "is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)"
            .to_string(),
        vec![("params[1]", user_id)],
    )
}

/// Messages Shape の where 句とパラメータの純粋組み立て
pub(crate) fn messages_shape_where(channel_id: &str) -> (String, Vec<(&'static str, &str)>) {
    (
        "channel_id = $1".to_string(),
        vec![("params[1]", channel_id)],
    )
}

/// Message Reactions Shape の where 句とパラメータの純粋組み立て
pub(crate) fn message_reactions_shape_where(
    channel_id: &str,
) -> (String, Vec<(&'static str, &str)>) {
    (
        "message_id IN (SELECT id FROM messages WHERE channel_id = $1 AND deleted_at IS NULL)"
            .to_string(),
        vec![("params[1]", channel_id)],
    )
}

/// Channel Users Shape の where 句とパラメータの純粋組み立て
pub(crate) fn channel_users_shape_where<'a>(
    channel_id: Option<&'a str>,
    user_id: &'a str,
) -> (String, Vec<(&'static str, &'a str)>) {
    if let Some(ch_id) = channel_id {
        ("channel_id = $1".to_string(), vec![("params[1]", ch_id)])
    } else {
        (
            "channel_id IN (SELECT id FROM channels WHERE is_private = false AND deleted_at IS NULL) OR channel_id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)".to_string(),
            vec![("params[1]", user_id)],
        )
    }
}

/// Electric へのストリーミング Shape プロキシ実行
async fn proxy_shape(
    state: &AppState,
    table: &str,
    tenant_id: &str,
    where_clause: Option<&str>,
    extra_params: &[(&str, &str)],
    raw_query: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let mut params = filter_electric_params(raw_query);
    params.push(("table".to_string(), table.to_string()));
    // params[2] is reserved for the server-selected tenant. Client SQL/params never pass through.
    let tenant_filter = if table == "users" {
        "id IN (SELECT user_id FROM tenant_users WHERE tenant_id = $2)"
    } else {
        "tenant_id = $2"
    };
    let scoped_where = match where_clause {
        Some(w) => format!("({tenant_filter}) AND ({w})"),
        None => tenant_filter.to_string(),
    };
    let (scoped_where, tenant_param) = if table == "users" {
        (scoped_where.replace("$2", "$1"), "params[1]")
    } else {
        (scoped_where, "params[2]")
    };
    params.push(("where".to_string(), scoped_where));
    params.push((tenant_param.to_string(), tenant_id.to_string()));
    for (k, v) in extra_params {
        params.push((k.to_string(), v.to_string()));
    }

    let url = format!(
        "{}/v1/shape",
        state.config.electric_url.trim_end_matches('/')
    );

    let mut req = state.http_client.get(&url).query(&params);

    if let Some(ref secret) = state.config.electric_secret {
        req = req.bearer_auth(secret);
    }

    let upstream_resp = req.send().await.map_err(|err| {
        tracing::error!("Failed to connect to Electric server at {url}: {err}");
        AppError::Internal(anyhow::anyhow!(
            "Failed to connect to Electric server: {err}"
        ))
    })?;

    let status =
        StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    // 転送ヘッダの抽出
    // - electric-* ヘッダーはすべて転送 (electric-up-to-date, electric-chunk-last-offset 含む)
    // - content-encoding, content-length はクライアント解凍とストリーミングのため除外
    let mut headers = HeaderMap::new();

    for (key, value) in upstream_resp.headers() {
        let key_str = key.as_str().to_lowercase();
        let should_forward = key_str.starts_with("electric-")
            || matches!(key_str.as_str(), "content-type" | "etag" | "cache-control");

        if should_forward
            && let Ok(name) = HeaderName::from_bytes(key.as_ref())
            && let Ok(val) = HeaderValue::from_bytes(value.as_bytes())
        {
            headers.insert(name, val);
        }
    }

    // Every shape is tenant-scoped; shared caches must never reuse its response.
    headers.insert(
        HeaderName::from_static("vary"),
        HeaderValue::from_static("Authorization, X-Dev-User, X-Tenant-ID"),
    );
    headers.insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static("private, no-cache"),
    );

    // ストリーミングボディをバッファリングせず透過的に転送
    let stream = upstream_resp.bytes_stream();
    let body = Body::from_stream(stream);

    let mut response = (status, body).into_response();
    *response.headers_mut() = headers;

    Ok(response)
}

/// Channels Shape
///
/// サブクエリと params[1] を使用することで、Electric が channel_user の変更に応じて
/// リアルタイムに行の move-in / move-out を配信できる (メンバーシップ変更時の 409 再取得を防止)。
async fn channels_shape(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let (where_clause, extra_params) = channels_shape_where(auth_user.id.as_str());

    proxy_shape(
        &state,
        "channels",
        &auth_user.tenant_id,
        Some(&where_clause),
        &extra_params,
        &raw_query,
    )
    .await
}

/// Messages Shape (channel_id 必須)
async fn messages_shape(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let channel_id = raw_query
        .get("channel_id")
        .ok_or_else(|| AppError::BadRequest("channel_id query parameter is required".into()))?;

    Uuid::parse_str(channel_id).map_err(|_| {
        AppError::BadRequest(format!("channel_id is not a valid UUID: {channel_id}"))
    })?;

    // チャンネル存在および閲覧権限チェック
    let mut db = state.db.clone();
    let channel = Channel::get_by_id(&mut db, channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found with id: {channel_id}"))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {channel_id}"
        )));
    }

    if channel.is_private {
        let is_member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?
        .is_some();

        // 存在推測攻撃を防ぐため 403 ではなく 404 を返却
        if !is_member {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {channel_id}"
            )));
        }
    }

    let (where_clause, extra_params) = messages_shape_where(channel_id.as_str());
    proxy_shape(
        &state,
        "messages",
        &auth_user.tenant_id,
        Some(&where_clause),
        &extra_params,
        &raw_query,
    )
    .await
}

/// Message Reactions Shape (channel_id 必須)
async fn message_reactions_shape(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let channel_id = raw_query
        .get("channel_id")
        .ok_or_else(|| AppError::BadRequest("channel_id query parameter is required".into()))?;

    Uuid::parse_str(channel_id).map_err(|_| {
        AppError::BadRequest(format!("channel_id is not a valid UUID: {channel_id}"))
    })?;

    // チャンネル存在および閲覧権限チェック
    let mut db = state.db.clone();
    let channel = Channel::get_by_id(&mut db, channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found with id: {channel_id}"))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {channel_id}"
        )));
    }

    if channel.is_private {
        let is_member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?
        .is_some();

        // 存在推測攻撃を防ぐため 403 ではなく 404 を返却
        if !is_member {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {channel_id}"
            )));
        }
    }

    let (where_clause, extra_params) = message_reactions_shape_where(channel_id.as_str());
    proxy_shape(
        &state,
        "message_reactions",
        &auth_user.tenant_id,
        Some(&where_clause),
        &extra_params,
        &raw_query,
    )
    .await
}

/// Users Shape (所属ワークスペース内のユーザーのみ同期)
async fn users_shape(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    proxy_shape(&state, "users", &auth_user.tenant_id, None, &[], &raw_query).await
}

/// Channel Users Shape (メンバー一覧)
async fn channel_users_shape(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let channel_id_opt = if let Some(channel_id) = raw_query.get("channel_id") {
        Uuid::parse_str(channel_id).map_err(|_| {
            AppError::BadRequest(format!("channel_id is not a valid UUID: {channel_id}"))
        })?;

        let mut db = state.db.clone();
        let channel = Channel::get_by_id(&mut db, channel_id.as_str())
            .await
            .map_err(|err| {
                if err.is_record_not_found() {
                    AppError::NotFound(format!("Channel not found with id: {channel_id}"))
                } else {
                    AppError::from(err)
                }
            })?;

        auth_user.check_channel(&channel)?;

        if channel.deleted_at.is_some() {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {channel_id}"
            )));
        }

        if channel.is_private {
            let is_member = ChannelUser::filter(
                ChannelUser::fields()
                    .channel_id()
                    .eq(channel_id.as_str())
                    .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
            )
            .first()
            .exec(&mut db)
            .await?
            .is_some();

            if !is_member {
                return Err(AppError::NotFound(format!(
                    "Channel not found with id: {channel_id}"
                )));
            }
        }
        Some(channel_id.as_str())
    } else {
        None
    };

    let (where_clause, extra_params) =
        channel_users_shape_where(channel_id_opt, auth_user.id.as_str());

    proxy_shape(
        &state,
        "channel_user",
        &auth_user.tenant_id,
        Some(&where_clause),
        &extra_params,
        &raw_query,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_electric_params() {
        let mut raw = HashMap::new();
        raw.insert("live".to_string(), "true".to_string());
        raw.insert("handle".to_string(), "12345".to_string());
        raw.insert("offset".to_string(), "-1".to_string());
        raw.insert("subset__tags".to_string(), "foo".to_string());
        // 許可外パラメータ (除外されるべき)
        raw.insert("table".to_string(), "users".to_string());
        raw.insert("where".to_string(), "1=1".to_string());
        raw.insert("token".to_string(), "secret".to_string());

        let filtered = filter_electric_params(&raw);
        let keys: Vec<String> = filtered.into_iter().map(|(k, _)| k).collect();

        assert!(keys.contains(&"live".to_string()));
        assert!(keys.contains(&"handle".to_string()));
        assert!(keys.contains(&"offset".to_string()));
        assert!(keys.contains(&"subset__tags".to_string()));

        assert!(!keys.contains(&"table".to_string()));
        assert!(!keys.contains(&"where".to_string()));
        assert!(!keys.contains(&"token".to_string()));
    }

    #[test]
    fn test_channels_shape_where() {
        let user_id = "user-123";
        let (clause, params) = channels_shape_where(user_id);

        assert!(clause.contains("is_private = false"));
        assert!(clause.contains("SELECT channel_id FROM channel_user WHERE user_id = $1"));
        assert_eq!(params, vec![("params[1]", "user-123")]);
    }

    #[test]
    fn test_messages_shape_where() {
        let channel_id = "channel-456";
        let (clause, params) = messages_shape_where(channel_id);

        assert_eq!(clause, "channel_id = $1");
        assert_eq!(params, vec![("params[1]", "channel-456")]);
    }

    #[test]
    fn test_channel_users_shape_where() {
        let user_id = "user-123";
        let channel_id = "channel-456";

        // channel_id 指定時
        let (clause, params) = channel_users_shape_where(Some(channel_id), user_id);
        assert_eq!(clause, "channel_id = $1");
        assert_eq!(params, vec![("params[1]", "channel-456")]);

        // channel_id 未指定時
        let (clause, params) = channel_users_shape_where(None, user_id);
        assert!(clause.contains("WHERE is_private = false AND deleted_at IS NULL"));
        assert!(clause.contains("WHERE user_id = $1"));
        assert_eq!(params, vec![("params[1]", "user-123")]);
    }

    #[test]
    fn test_message_reactions_shape_where() {
        let channel_id = "channel-456";
        let (clause, params) = message_reactions_shape_where(channel_id);

        assert_eq!(
            clause,
            "message_id IN (SELECT id FROM messages WHERE channel_id = $1 AND deleted_at IS NULL)"
        );
        assert_eq!(params, vec![("params[1]", "channel-456")]);
    }
}
