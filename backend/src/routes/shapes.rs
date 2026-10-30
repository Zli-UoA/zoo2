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
        .route(
            "/api/shapes/message_reactions",
            get(message_reactions_shape),
        )
        .route("/api/shapes/users", get(users_shape))
        .route("/api/shapes/channel_users", get(channel_users_shape))
        .route("/api/shapes/notifications", get(notifications_shape))
        .route("/api/shapes/attachments", get(attachments_shape))
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

/// Notifications Shape の where 句とパラメータの純粋組み立て
///
/// [P1] ユーザー本人の通知であることに加え、現在閲覧権限があるチャンネル (公開チャンネル、
/// または所属しているプライベートチャンネル) の通知のみを配信する。
/// また、論理削除されたメッセージ (deleted_at IS NOT NULL) およびチャンネルの通知を除外する。
/// キック・退出やメッセージ・チャンネル削除が行われた場合、ElectricSQL の move-out 機構によりクライアントから即座に通知が除外される。
pub(crate) fn notifications_shape_where(user_id: &str) -> (String, Vec<(&'static str, &str)>) {
    (
        "user_id = $1 AND message_id IN (SELECT id FROM messages WHERE deleted_at IS NULL) AND channel_id IN (SELECT id FROM channels WHERE deleted_at IS NULL AND (is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)))"
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

/// Attachments Shape の where 句とパラメータの純粋組み立て
///
/// 親メッセージが紐付けられており (message_id IS NOT NULL)、かつ親メッセージが論理削除されていない
/// (deleted_at IS NULL) 添付ファイルのみを配信する。親メッセージ削除時は move-out でクライアントから除外される。
pub(crate) fn attachments_shape_where(channel_id: &str) -> (String, Vec<(&'static str, &str)>) {
    (
        "channel_id = $1 AND message_id IN (SELECT id FROM messages WHERE channel_id = $1 AND deleted_at IS NULL)".to_string(),
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
    where_clause: Option<&str>,
    extra_params: &[(&str, &str)],
    raw_query: &HashMap<String, String>,
    is_user_scoped: bool,
    actor: &str,
) -> Result<Response, AppError> {
    let mut params = filter_electric_params(raw_query);
    params.push(("table".to_string(), table.to_string()));
    if table == "messages"
        && raw_query
            .get("_app_metadata_allowed")
            .is_some_and(|v| v == "false")
    {
        params.push(("columns".into(),"id,user_id,channel_id,content,card,created_at,updated_at,deleted_at,parent_id,app_blocks,app_attachments,app_unfurls,app_origin_id,message_kind,broadcast".into()));
    }

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

    let mut req = state.electric_client.get(&url).query(&params);

    if let Some(ref secret) = state.config.electric_secret {
        // Electric の API secret は Bearer ではなくクエリで渡す。
        req = req.query(&[("secret", secret)]);
    }

    let upstream_resp = req.send().await.map_err(|err| {
        // URL のクエリに含まれる API secret をログやエラー応答へ出さない。
        let err = err.without_url();
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
    use futures_util::StreamExt;
    let mut db = state.db.clone();
    let initial_guest = crate::apps::events::guest_in_db(&mut db, actor).await?;
    let channel = raw_query.get("channel_id").cloned();
    let stream = upstream_resp.bytes_stream().boxed();
    let stream = futures_util::stream::unfold(
        (
            stream,
            state.clone(),
            actor.to_owned(),
            initial_guest,
            channel,
        ),
        |(mut upstream, state, actor, initial_guest, channel)| async move {
            loop {
                let mut db = state.db.clone();
                let mut tx = db.transaction().await.ok()?;
                let access=crate::apps::one(&mut tx,"SELECT json_build_object('ok',NOT u.is_deactivated AND u.is_guest=$2::boolean AND ($3='' OR EXISTS(SELECT 1 FROM channels c WHERE c.id=$3 AND c.deleted_at IS NULL AND (NOT c.is_private AND NOT c.is_dm AND NOT u.is_guest OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=u.id)))))::text FROM users u WHERE u.id=$1",&[&actor,if initial_guest{"true"}else{"false"},channel.as_deref().unwrap_or("")]).await.ok()?;
                if access["ok"] != true {
                    return None;
                }
                drop(tx);
                tokio::select! {chunk=upstream.next()=>return chunk.map(|chunk|(chunk,(upstream,state,actor,initial_guest,channel))),_=tokio::time::sleep(std::time::Duration::from_secs(1))=>{}}
            }
        },
    );
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
    let (mut where_clause, extra_params) = channels_shape_where(auth_user.id.as_str());
    let mut db = state.db.clone();
    if crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
        where_clause = "id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)".into();
    }

    proxy_shape(
        &state,
        "channels",
        Some(&where_clause),
        &extra_params,
        &raw_query,
        true,
        &auth_user.id,
    )
    .await
}

/// Messages Shape (channel_id 必須)
pub(crate) async fn messages_shape(
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

    if channel.is_private || crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
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
        Some(&where_clause),
        &extra_params,
        &raw_query,
        channel.is_private,
        &auth_user.id,
    )
    .await
}

/// Message Reactions Shape (channel_id 必須)
pub(crate) async fn message_reactions_shape(
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

    if channel.is_private || crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
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
        Some(&where_clause),
        &extra_params,
        &raw_query,
        channel.is_private,
        &auth_user.id,
    )
    .await
}

/// Users Shape (全ユーザー同期)
pub(crate) async fn users_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let mut db = state.db.clone();
    if crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
        return proxy_shape(&state,"users",Some("id = $1 OR id IN (SELECT user_id FROM channel_user WHERE channel_id IN (SELECT channel_id FROM channel_user WHERE user_id = $1))"),&[("params[1]",auth_user.id.as_str())],&raw_query,true,&auth_user.id).await;
    }
    proxy_shape(&state, "users", None, &[], &raw_query, false, &auth_user.id).await
}

/// Channel Users Shape (メンバー一覧)
pub(crate) async fn channel_users_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let (channel_id_opt, is_private) = if let Some(channel_id) = raw_query.get("channel_id") {
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

        if channel.is_private || crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
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
        (Some(channel_id.as_str()), channel.is_private)
    } else {
        (None, true)
    };

    let (mut where_clause, extra_params) =
        channel_users_shape_where(channel_id_opt, auth_user.id.as_str());
    if channel_id_opt.is_none() {
        let mut db = state.db.clone();
        if crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
            where_clause =
                "channel_id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)".into();
        }
    }

    proxy_shape(
        &state,
        "channel_user",
        Some(&where_clause),
        &extra_params,
        &raw_query,
        is_private,
        &auth_user.id,
    )
    .await
}

/// Notifications Shape (ユーザー別認可・配信)
async fn notifications_shape(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let (mut where_clause, extra_params) = notifications_shape_where(auth_user.id.as_str());
    let mut db = state.db.clone();
    if crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
        where_clause = where_clause.replace("is_private = false OR ", "");
    }

    proxy_shape(
        &state,
        "notifications",
        Some(&where_clause),
        &extra_params,
        &raw_query,
        true,
        &auth_user.id,
    )
    .await
}

/// App tokens only enumerate channels their acting identity has joined.
pub(crate) async fn app_channels_shape(
    state: AppState,
    p: &crate::apps::Principal,
    params: HashMap<String, String>,
) -> Result<Response, AppError> {
    proxy_shape(
        &state,
        "channels",
        Some(
            "deleted_at IS NULL AND id IN (SELECT channel_id FROM channel_user WHERE user_id = $1)",
        ),
        &[("params[1]", p.user_id.as_str())],
        &params,
        true,
        &p.user_id,
    )
    .await
}

/// Attachments Shape (channel_id 必須)
async fn attachments_shape(
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
        .await?;

        if is_member.is_none() {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {channel_id}"
            )));
        }
    }

    let (where_clause, extra_params) = attachments_shape_where(channel_id.as_str());

    proxy_shape(
        &state,
        "attachments",
        Some(&where_clause),
        &extra_params,
        &raw_query,
        false,
        &auth_user.id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attachments_shape_where() {
        let (clause, params) = attachments_shape_where("ch-123");
        assert_eq!(
            clause,
            "channel_id = $1 AND message_id IN (SELECT id FROM messages WHERE channel_id = $1 AND deleted_at IS NULL)"
        );
        assert_eq!(params, vec![("params[1]", "ch-123")]);
    }

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
    fn test_notifications_shape_where() {
        let user_id = "user-123";
        let (clause, params) = notifications_shape_where(user_id);

        assert!(clause.contains("user_id = $1"));
        assert!(
            clause.contains("message_id IN (SELECT id FROM messages WHERE deleted_at IS NULL)")
        );
        assert!(clause.contains("deleted_at IS NULL"));
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
