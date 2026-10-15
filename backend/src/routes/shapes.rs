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
    auth::AuthUser,
    error::AppError,
    models::{Channel, ChannelUser},
    state::AppState,
};

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
    extra_params: &[(&str, &str)],
    raw_query: &HashMap<String, String>,
    is_user_scoped: bool,
) -> Result<Response, AppError> {
    let mut params = filter_electric_params(raw_query);
    params.push(("table".to_string(), table.to_string()));
    if let Some(w) = where_clause {
        params.push(("where".to_string(), w.to_string()));
    }
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

    // ユーザー別認可フィルタが適用された Shape の場合、共有キャッシュによる情報漏洩を防止
    if is_user_scoped {
        headers.insert(
            HeaderName::from_static("vary"),
            HeaderValue::from_static("Authorization, X-Dev-User"),
        );
        headers.insert(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("private, no-cache"),
        );
    }

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
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let where_clause =
        "is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)";
    let extra_params = [("params[1]", auth_user.id.as_str())];

    proxy_shape(
        &state,
        "channels",
        Some(where_clause),
        &extra_params,
        &raw_query,
        true,
    )
    .await
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

    let where_clause = "channel_id = $1";
    let extra_params = [("params[1]", channel_id.as_str())];
    proxy_shape(
        &state,
        "messages",
        Some(where_clause),
        &extra_params,
        &raw_query,
        channel.is_private,
    )
    .await
}

/// Users Shape (全ユーザー同期)
async fn users_shape(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    proxy_shape(&state, "users", None, &[], &raw_query, false).await
}

/// Channel Users Shape (メンバー一覧)
async fn channel_users_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let (where_clause, extra_params, is_private): (String, Vec<(&str, &str)>, bool) = if let Some(
        channel_id,
    ) =
        raw_query.get("channel_id")
    {
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
        (
            "channel_id = $1".to_string(),
            vec![("params[1]", channel_id.as_str())],
            channel.is_private,
        )
    } else {
        // channel_id 未指定の場合: 公開チャンネルまたは所属チャンネルのメンバー情報
        // サブクエリを使うことで、公開チャンネル作成時や参加チャンネル変更時に
        // 全ユーザーの shape が 409 でフルリシンクされるのを防止
        (
                "channel_id IN (SELECT id FROM channels WHERE is_private = false AND deleted_at IS NULL) OR channel_id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)".to_string(),
                vec![("params[1]", auth_user.id.as_str())],
                true,
            )
    };

    proxy_shape(
        &state,
        "channel_user",
        Some(&where_clause),
        &extra_params,
        &raw_query,
        is_private,
    )
    .await
}
