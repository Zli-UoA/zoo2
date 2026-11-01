//! 各ユーザーによる Slack アカウントのセルフサービス連携 (#291)。
//!
//! 1. フロントが `POST /api/slack/oauth/start` で認可 URL を受け取り、Slack へ移動する
//! 2. Slack は `GET /api/slack/oauth/callback` に戻す。ここではトークン交換をせず、
//!    `code` と `state` を付けてフロントの設定画面へリダイレクトするだけにする
//! 3. フロントがログイン中ユーザーとして `POST /api/slack/oauth/complete` を呼ぶ。
//!    state のユーザーと呼び出したユーザーが一致するときだけ連携する。
//!    (他人が作った認可 URL を踏まされても、その人のアカウントに連携されない)
//! 4. 連携したら、ZIP 取り込みで作ったプレースホルダの投稿などを本人へ付け替える

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Query, State},
    response::Redirect,
    routing::{get, post},
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::crypto::{OAuthState, decrypt, encrypt, sign_state, verify_state};
use crate::{
    apps::{human, one, query},
    auth::{AuthUser, rbac::is_workspace_admin},
    error::AppError,
    state::AppState,
};

/// RFC 3986 の非予約文字以外をエンコードする
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
const AUTHORIZE_URL: &str = "https://slack.com/oauth/v2/authorize";
const STATE_TTL_SECS: i64 = 600;

/// 差分同期 (#292) とブリッジ (#293) で使うユーザースコープ
pub const USER_SCOPES: &[&str] = &[
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    "users:read",
    "reactions:read",
    "files:read",
    "chat:write",
];
pub const BOT_SCOPES: &[&str] = &[
    "channels:read",
    "users:read",
    "files:read",
    "chat:write",
    "chat:write.customize",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/slack/link", get(status).delete(unlink))
        .route("/api/slack/oauth/start", post(start))
        .route("/api/slack/oauth/callback", get(callback))
        .route("/api/slack/oauth/complete", post(complete))
        .route("/api/admin/slack/manifest", get(manifest))
        .layer(axum::middleware::map_response(crate::apps::no_store))
}

fn key(s: &AppState) -> Result<[u8; 32], AppError> {
    if !s.config.slack.oauth_enabled() {
        return Err(AppError::NotFound("Slack linking is not configured".into()));
    }
    s.config
        .slack
        .encryption_key()
        .ok_or_else(|| AppError::NotFound("Slack linking is not configured".into()))
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

async fn link_state(
    tx: &mut toasty::Transaction<'_>,
    s: &AppState,
    user: &str,
) -> Result<Value, AppError> {
    let link = query(
        tx,
        "SELECT json_build_object('team_id',team_id,'team_name',team_name,'slack_user_id',slack_user_id,\
         'slack_user_name',slack_user_name,'linked_at',created_at)::text FROM slack_user_links WHERE user_id=$1",
        &[user],
    )
    .await?
    .into_iter()
    .next();
    Ok(json!({
        "configured": s.config.slack.oauth_enabled(),
        "link": link.unwrap_or(Value::Null),
    }))
}

async fn status(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let state = link_state(&mut tx, &s, &user.id).await?;
    tx.commit().await?;
    Ok(Json(state))
}

async fn start(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let key = key(&s)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &user).await?;
    tx.commit().await?;
    let slack = &s.config.slack;
    let state = sign_state(
        &key,
        &OAuthState {
            user_id: user.id.clone(),
            nonce: uuid::Uuid::new_v4().simple().to_string(),
            expires_at: now() + STATE_TTL_SECS,
        },
    );
    let encode = |v: &str| utf8_percent_encode(v, QUERY_VALUE).to_string();
    let mut url = format!(
        "{AUTHORIZE_URL}?client_id={}&user_scope={}&redirect_uri={}&state={}",
        encode(slack.client_id.as_deref().unwrap_or_default()),
        encode(&USER_SCOPES.join(",")),
        encode(slack.oauth_redirect_url.as_deref().unwrap_or_default()),
        encode(&state),
    );
    if let Some(team) = slack.team_id.as_deref() {
        url.push_str(&format!("&team={}", encode(team)));
    }
    Ok(Json(json!({ "authorize_url": url })))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Slack からの戻り。トークンは交換せず、フロントの設定画面へ渡す。
async fn callback(State(s): State<AppState>, Query(q): Query<CallbackQuery>) -> Redirect {
    let base = format!("{}/settings?tab=slack", s.config.slack.frontend_url);
    let encode = |v: &str| utf8_percent_encode(v, QUERY_VALUE).to_string();
    match (q.code, q.state, q.error) {
        (Some(code), Some(state), None) => Redirect::to(&format!(
            "{base}&code={}&state={}",
            encode(&code),
            encode(&state)
        )),
        (_, _, Some(error)) if error == "access_denied" => {
            Redirect::to(&format!("{base}&slack=error&reason=denied"))
        }
        _ => Redirect::to(&format!("{base}&slack=error&reason=failed")),
    }
}

#[derive(Deserialize)]
struct CompleteRequest {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct AccessResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    authed_user: Option<AuthedUser>,
    #[serde(default)]
    team: Option<Team>,
}

#[derive(Deserialize)]
struct AuthedUser {
    id: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct Team {
    id: String,
    #[serde(default)]
    name: String,
}

fn http() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(e.into()))
}

async fn complete(
    State(s): State<AppState>,
    user: AuthUser,
    Json(body): Json<CompleteRequest>,
) -> Result<Json<Value>, AppError> {
    let key = key(&s)?;
    let state = verify_state(&key, &body.state, now())
        .ok_or_else(|| AppError::BadRequest("invalid_state".into()))?;
    if state.user_id != user.id {
        return Err(AppError::BadRequest("invalid_state".into()));
    }
    let slack = &s.config.slack;

    // トークンを交換する (client_id / client_secret は Basic 認証で送る)
    let encode = |v: &str| utf8_percent_encode(v, QUERY_VALUE).to_string();
    let response = http()?
        .post(format!("{}/oauth.v2.access", slack.api_base_url))
        .basic_auth(
            slack.client_id.as_deref().unwrap_or_default(),
            slack.client_secret.as_deref(),
        )
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "code={}&redirect_uri={}",
            encode(&body.code),
            encode(slack.oauth_redirect_url.as_deref().unwrap_or_default())
        ))
        .send()
        .await
        .map_err(|e| AppError::BadGateway(format!("Slack token exchange failed: {e}")))?
        .json::<AccessResponse>()
        .await
        .map_err(|e| AppError::BadGateway(format!("Invalid Slack token response: {e}")))?;
    if !response.ok {
        return Err(AppError::BadGateway(format!(
            "Slack token exchange failed: {}",
            response.error.unwrap_or_default()
        )));
    }
    let (Some(authed), Some(team)) = (response.authed_user, response.team) else {
        return Err(AppError::BadGateway(
            "Slack did not return a user token".into(),
        ));
    };
    let Some(token) = authed.access_token.filter(|t| !t.is_empty()) else {
        return Err(AppError::BadGateway(
            "Slack did not return a user token".into(),
        ));
    };
    if slack.team_id.as_deref().is_some_and(|t| t != team.id) {
        discard(&s, &token).await;
        return Err(AppError::Forbidden("team_mismatch".into()));
    }
    let slack_user_name = user_name(&s, &token, &authed.id).await;

    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &user).await?;
    let result = link_account(
        &mut tx,
        &user.id,
        &team,
        &authed.id,
        &slack_user_name,
        &encrypt(&key, &token)?,
        &authed.scope,
    )
    .await;
    if let Err(e) = result {
        drop(tx);
        discard(&s, &token).await;
        return Err(e);
    }
    let state = link_state(&mut tx, &s, &user.id).await?;
    tx.commit().await?;
    Ok(Json(state))
}

/// Slack の表示名 (取れなければ空)
async fn user_name(s: &AppState, token: &str, slack_user: &str) -> String {
    let Ok(client) = http() else {
        return String::new();
    };
    let Ok(response) = client
        .get(format!("{}/users.info", s.config.slack.api_base_url))
        .query(&[("user", slack_user)])
        .bearer_auth(token)
        .send()
        .await
    else {
        return String::new();
    };
    let value: Value = response.json().await.unwrap_or(Value::Null);
    let profile = &value["user"]["profile"];
    [
        profile["display_name"].as_str(),
        profile["real_name"].as_str(),
        value["user"]["name"].as_str(),
    ]
    .into_iter()
    .flatten()
    .find(|n| !n.trim().is_empty())
    .unwrap_or_default()
    .to_string()
}

/// トークンを失効させる。すでに無効なトークンも失効済みとして扱う。
async fn revoke(s: &AppState, token: &str) -> Result<(), AppError> {
    let failed =
        |detail: String| AppError::BadGateway(format!("Slack token revocation failed: {detail}"));
    let response = http()?
        .post(format!("{}/auth.revoke", s.config.slack.api_base_url))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| failed(e.to_string()))?;
    if !response.status().is_success() {
        return Err(failed(response.status().to_string()));
    }
    let body: Value = response.json().await.map_err(|e| failed(e.to_string()))?;
    let already_invalid = matches!(
        body["error"].as_str(),
        Some("invalid_auth" | "token_revoked" | "account_inactive" | "not_authed")
    );
    if body["ok"] == true || already_invalid {
        return Ok(());
    }
    Err(failed(
        body["error"].as_str().unwrap_or("unknown").to_string(),
    ))
}

/// 連携できなかったときに受け取ったトークンを片付ける (失敗しても連携の失敗理由を優先する)
async fn discard(s: &AppState, token: &str) {
    if let Err(e) = revoke(s, token).await {
        tracing::warn!(error = %e, "Failed to revoke an unused Slack token");
    }
}

async fn exec(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<(), AppError> {
    let mut statement = toasty::sql::statement(sql);
    for value in params {
        statement = statement.bind(*value);
    }
    statement.exec(tx).await?;
    Ok(())
}

async fn link_account(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    team: &Team,
    slack_user: &str,
    slack_user_name: &str,
    token_enc: &str,
    scopes: &str,
) -> Result<(), AppError> {
    let conflict = || AppError::Conflict("already_linked".into());
    // 同じ Slack アカウントが別の Zoo ユーザーに連携済み (または以前連携していた) なら拒否する
    let owner = query(
        tx,
        "SELECT json_build_object('user_id',user_id)::text FROM slack_user_links WHERE team_id=$1 AND slack_user_id=$2 \
         UNION ALL SELECT json_build_object('user_id',user_id)::text FROM slack_users \
         WHERE team_id=$1 AND slack_user_id=$2 AND linked",
        &[&team.id, slack_user],
    )
    .await?;
    if owner.iter().any(|o| o["user_id"] != user) {
        return Err(conflict());
    }

    exec(
        tx,
        "INSERT INTO slack_user_links(user_id,team_id,team_name,slack_user_id,slack_user_name,access_token_enc,scopes) \
         VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (user_id) DO UPDATE SET team_id=EXCLUDED.team_id,\
         team_name=EXCLUDED.team_name,slack_user_id=EXCLUDED.slack_user_id,slack_user_name=EXCLUDED.slack_user_name,\
         access_token_enc=EXCLUDED.access_token_enc,scopes=EXCLUDED.scopes,updated_at=now()",
        &[user, &team.id, &team.name, slack_user, slack_user_name, token_enc, scopes],
    )
    .await
    .map_err(|e| match e {
        AppError::Conflict(_) => conflict(),
        other => other,
    })?;
    reassign(tx, &team.id, slack_user, slack_user_name, user).await
}

/// プレースホルダ `from` の投稿・リアクション・参加チャンネル・添付を `to` へ移す。
/// 何度呼んでも同じ結果になる (残っていた分だけが移る)。
pub async fn move_content(
    tx: &mut toasty::Transaction<'_>,
    from: &str,
    to: &str,
) -> Result<(), AppError> {
    if from == to {
        return Ok(());
    }
    // 付け替えで App へイベントを大量配送しない
    one(
        tx,
        "SELECT json_build_object('ok',set_config('zoo.suppress_app_events','on',true))::text",
        &[],
    )
    .await?;
    for sql in [
        "UPDATE messages SET user_id=$2 WHERE user_id=$1",
        // 本人が同じ絵文字を付けていれば重複を消してから移す
        "DELETE FROM message_reactions r WHERE r.user_id=$1 AND EXISTS(SELECT 1 FROM message_reactions x \
         WHERE x.message_id=r.message_id AND x.user_id=$2 AND x.emoji=r.emoji)",
        "UPDATE message_reactions SET user_id=$2 WHERE user_id=$1",
        // 新しく参加するチャンネルの過去ログは既読にする
        "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) \
         SELECT $2,cu.channel_id,m.latest FROM channel_user cu \
         CROSS JOIN LATERAL (SELECT max(created_at) AS latest FROM messages WHERE channel_id=cu.channel_id) m \
         WHERE cu.user_id=$1 AND m.latest IS NOT NULL \
         AND NOT EXISTS(SELECT 1 FROM channel_user mine WHERE mine.user_id=$2 AND mine.channel_id=cu.channel_id) \
         ON CONFLICT (user_id,channel_id) DO NOTHING",
        "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $2,channel_id,joined_at \
         FROM channel_user WHERE user_id=$1 ON CONFLICT DO NOTHING",
        "DELETE FROM channel_user WHERE user_id=$1",
        "UPDATE attachments SET uploader_id=$2 WHERE uploader_id=$1",
        "UPDATE slack_files SET uploader_id=$2 WHERE uploader_id=$1",
        "UPDATE home_posts SET user_id=$2 WHERE user_id=$1",
    ] {
        exec(tx, sql, &[from, to]).await?;
    }
    // プレースホルダの users 行は残す (他の表から参照されている可能性があるため)。無効化済みのまま。
    Ok(())
}

/// ZIP 取り込みで作ったプレースホルダの投稿・リアクション・参加チャンネルを本人へ移す。
/// プレースホルダがまだなければ、後の取り込みが最初から本人の投稿になるよう対応だけ作る。
///
/// 取り込みの途中で連携した場合、ワーカーがキャッシュしたプレースホルダへ書き込みが続くことがある。
/// そのためプレースホルダは決定的 ID (`slack::user_id`) で特定し、再連携や取り込み完了時の
/// [`reconcile_team`] でも残りを回収できるようにしている。
pub async fn reassign(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_user: &str,
    slack_user_name: &str,
    user: &str,
) -> Result<(), AppError> {
    let mapping = query(
        tx,
        "SELECT json_build_object('user_id',user_id,'linked',linked)::text FROM slack_users \
         WHERE team_id=$1 AND slack_user_id=$2 FOR UPDATE",
        &[team, slack_user],
    )
    .await?
    .into_iter()
    .next();
    match mapping {
        None => {
            exec(
                tx,
                "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name) VALUES($1,$2,$3,true,$4)",
                &[team, slack_user, user, slack_user_name],
            )
            .await?;
        }
        Some(m) if m["linked"] == true && m["user_id"] != user => {
            return Err(AppError::Conflict("already_linked".into()));
        }
        Some(_) => {
            exec(
                tx,
                "UPDATE slack_users SET user_id=$3,linked=true,updated_at=now() WHERE team_id=$1 AND slack_user_id=$2",
                &[team, slack_user, user],
            )
            .await?;
        }
    }
    move_content(tx, &super::user_id(team, slack_user), user).await
}

/// 連携済みの Slack ユーザーについて、プレースホルダに残った分を本人へ移す。
/// 取り込みと添付取得の最後に呼び、取り込み中に連携した人の書き込みも回収する。
pub async fn reconcile_team(tx: &mut toasty::Transaction<'_>, team: &str) -> Result<(), AppError> {
    let linked = query(
        tx,
        "SELECT json_build_object('slack_user_id',slack_user_id,'user_id',user_id)::text \
         FROM slack_users WHERE team_id=$1 AND linked",
        &[team],
    )
    .await?;
    for row in linked {
        let slack_user = row["slack_user_id"].as_str().unwrap_or_default();
        let user = row["user_id"].as_str().unwrap_or_default();
        move_content(tx, &super::user_id(team, slack_user), user).await?;
    }
    Ok(())
}

async fn unlink(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let link = query(
        &mut tx,
        "SELECT json_build_object('token',access_token_enc)::text FROM slack_user_links WHERE user_id=$1 FOR UPDATE",
        &[&user.id],
    )
    .await?
    .into_iter()
    .next();
    if let Some(link) = link {
        // 失効を確かめてから削除する。失敗したらトークンを残し、もう一度解除して再試行できるようにする。
        // 鍵を変えて復号できないトークンは失効させようがないので、そのまま削除する。
        match s
            .config
            .slack
            .encryption_key()
            .map(|key| decrypt(&key, link["token"].as_str().unwrap_or_default()))
        {
            Some(Ok(token)) => revoke(&s, &token).await?,
            _ => {
                tracing::warn!(user_id = %user.id, "Deleting a Slack token that cannot be decrypted")
            }
        }
        exec(
            &mut tx,
            "DELETE FROM slack_user_links WHERE user_id=$1",
            &[&user.id],
        )
        .await?;
    }
    let state = link_state(&mut tx, &s, &user.id).await?;
    tx.commit().await?;
    // 付け替えた投稿と slack_users の対応はそのまま残す (同じ人が再連携できる)
    Ok(Json(state))
}

/// ワークスペースで作る Slack アプリの App Manifest
async fn manifest(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    if !is_workspace_admin(&mut tx, &user.id, s.config.dev_auth_enabled).await? {
        return Err(AppError::Forbidden(
            "Only workspace admins can view the manifest".into(),
        ));
    }
    tx.commit().await?;
    let redirect = s
        .config
        .slack
        .oauth_redirect_url
        .clone()
        .unwrap_or_else(|| "https://<zoo-backend>/api/slack/oauth/callback".into());
    Ok(Json(json!({
        "configured": s.config.slack.oauth_enabled(),
        "manifest": {
            "display_information": {
                "name": "Zoo",
                "description": "Zoo と Slack を連携します",
            },
            "features": {
                "bot_user": { "display_name": "Zoo", "always_online": false },
            },
            "oauth_config": {
                "redirect_urls": [redirect],
                "scopes": { "user": USER_SCOPES, "bot": BOT_SCOPES },
            },
            "settings": {
                "org_deploy_enabled": false,
                "socket_mode_enabled": false,
                "token_rotation_enabled": false,
            },
        },
    })))
}
