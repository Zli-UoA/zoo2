use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::AppError,
    models::{Channel, ChannelUser},
    state::AppState,
};

#[derive(Debug, serde::Deserialize)]
pub struct MessagesShapeQuery {
    pub channel_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChannelUsersShapeQuery {
    pub channel_id: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/shapes/channels", get(channels_shape))
        .route("/api/shapes/messages", get(messages_shape))
        .route("/api/shapes/users", get(users_shape))
        .route("/api/shapes/channel_users", get(channel_users_shape))
}

/// クライアントから送信されたクエリパラメータのうち、Electric プロトコルで許可されたものを転送
fn filter_electric_params(raw_query: &HashMap<String, String>) -> Vec<(String, String)> {
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

/// Electric へのストリーミング Shape プロキシ実行
async fn proxy_shape(
    state: &AppState,
    table: &str,
    where_clause: Option<&str>,
    raw_query: &HashMap<String, String>,
) -> Result<Response, AppError> {
    let mut params = filter_electric_params(raw_query);
    params.push(("table".to_string(), table.to_string()));
    if let Some(w) = where_clause {
        params.push(("where".to_string(), w.to_string()));
    }

    let url = format!("{}/v1/shape", state.config.electric_url.trim_end_matches('/'));

    let mut req = state.http_client.get(&url).query(&params);

    if let Some(ref secret) = state.config.electric_secret {
        req = req.bearer_auth(secret);
    }

    let upstream_resp = req.send().await.map_err(|err| {
        tracing::error!("Failed to connect to Electric server at {url}: {err}");
        AppError::Internal(anyhow::anyhow!("Failed to connect to Electric server: {err}"))
    })?;

    let status = StatusCode::from_u16(upstream_resp.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);

    // 転送ヘッダの抽出 (content-encoding, content-length は reqwest が自動展開するため除外)
    let mut headers = HeaderMap::new();
    let forward_headers = [
        "electric-handle",
        "electric-offset",
        "electric-schema",
        "electric-cursor",
        "content-type",
        "cache-control",
        "etag",
    ];

    for (key, value) in upstream_resp.headers() {
        let key_str = key.as_str().to_lowercase();
        if forward_headers.contains(&key_str.as_str()) {
            if let Ok(name) = HeaderName::from_bytes(key.as_ref()) {
                if let Ok(val) = HeaderValue::from_bytes(value.as_bytes()) {
                    headers.insert(name, val);
                }
            }
        }
    }

    // ストリーミングボディをバッファリングせず透過的に転送
    let stream = upstream_resp.bytes_stream();
    let body = Body::from_stream(stream);

    let mut response = (status, body).into_response();
    *response.headers_mut() = headers;

    Ok(response)
}

/// Channels Shape
async fn channels_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    // 公開チャンネルまたは自分が所属しているプライベートチャンネルのみ
    // (auth_user.id は UUID 形式検証済み)
    let where_clause = format!(
        "is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = '{}')",
        auth_user.id
    );

    proxy_shape(&state, "channels", Some(&where_clause), &raw_query).await
}

/// Messages Shape (channel_id 必須)
async fn messages_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
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
            return Err(AppError::Forbidden(
                "You do not have access to this private channel".into(),
            ));
        }
    }

    let where_clause = format!("channel_id = '{channel_id}'");
    proxy_shape(&state, "messages", Some(&where_clause), &raw_query).await
}

/// Users Shape (全ユーザー同期)
async fn users_shape(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    proxy_shape(&state, "users", None, &raw_query).await
}

/// Channel Users Shape (メンバー一覧)
async fn channel_users_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let where_clause = if let Some(channel_id) = raw_query.get("channel_id") {
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
                return Err(AppError::Forbidden(
                    "You do not have access to this private channel".into(),
                ));
            }
        }
        Some(format!("channel_id = '{channel_id}'"))
    } else {
        // channel_id 未指定の場合、公開チャンネルまたは所属チャンネルのメンバーのみ
        Some(format!(
            "channel_id IN (SELECT id FROM channels WHERE is_private = false) OR channel_id IN (SELECT channel_id FROM channel_user WHERE user_id = '{}')",
            auth_user.id
        ))
    };

    proxy_shape(&state, "channel_user", where_clause.as_deref(), &raw_query).await
}
