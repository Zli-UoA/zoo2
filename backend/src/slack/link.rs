//! 各ユーザーによる Slack アカウントのセルフサービス連携 (#291)。
//!
//! 1. フロントが `POST /api/slack/oauth/start` で認可 URL を受け取り、Slack へ移動する
//! 2. Slack は `GET /api/slack/oauth/callback` に戻す。ここではトークン交換をせず、
//!    `code` と `state` を付けてフロントの設定画面へリダイレクトするだけにする
//! 3. フロントがログイン中ユーザーとして `POST /api/slack/oauth/complete` を呼ぶ。
//!    state のユーザーと呼び出したユーザーが一致するときだけ連携する。
//!    (他人が作った認可 URL を踏まされても、その人のアカウントに連携されない)
//! 4. 連携したら、ZIP 取り込みで作ったプレースホルダの投稿などを本人へ付け替える

use std::{collections::HashMap, time::Duration};

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

/// アカウント連携で求めるユーザースコープ。いまは連携時の表示名の取得 (`users.info`) にだけ使う。
/// 差分同期 (#292) とブリッジ (#293) で必要なスコープは、その機能を入れるときに足し、
/// 各ユーザーに連携し直してもらう (`slack_user_links.scopes` で、まだの人を判別できる)。
pub const USER_SCOPES: &[&str] = &["users:read"];
/// manifest の bot_user に付けるスコープ。このアプリの Bot Token を `SLACK_IMPORT_BOT_TOKEN` に
/// 使う場合のために `files:read` だけ求める (bot_user を宣言するので空にはしない)。
pub const BOT_SCOPES: &[&str] = &["files:read"];

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
    .next()
    // SLACK_TEAM_ID を後から変えた場合も、解除できるよう連携は返し、使えないことだけを示す
    .map(|mut link| {
        let allowed = s
            .config
            .slack
            .team_allowed(link["team_id"].as_str().unwrap_or_default());
        link["team_allowed"] = allowed.into();
        link
    });
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
    if !slack.team_allowed(&team.id) {
        discard(&s, &team.id, &authed.id, &token).await;
        return Err(AppError::Forbidden("team_mismatch".into()));
    }

    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &user).await?;
    // 同じ Slack アカウントの解除や、ほかの連携の失敗による失効と直列にする。ロックを取ってから
    // トークンが有効か確かめ、交換から保存までの間に失効した同じトークンを保存しない
    lock_account(&mut tx, &team.id, &authed.id).await?;
    let Some(slack_user_name) = user_name(&s, &token, &authed.id).await else {
        return Err(AppError::BadGateway(
            "The Slack token was revoked while linking; try again".into(),
        ));
    };
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
        // アカウントのロックを放してから片付ける (discard が同じロックを取り直す)
        let _ = tx.rollback().await;
        discard(&s, &team.id, &authed.id, &token).await;
        return Err(e);
    }
    let state = link_state(&mut tx, &s, &user.id).await?;
    tx.commit().await?;
    Ok(Json(state))
}

/// Slack が「トークンが無効」と答えたときのエラー
const INVALID_TOKEN_ERRORS: &[&str] = &[
    "invalid_auth",
    "token_revoked",
    "account_inactive",
    "not_authed",
];

/// Slack の表示名 (取れなければ空)。トークンが無効 (失効済みなど) なら None
async fn user_name(s: &AppState, token: &str, slack_user: &str) -> Option<String> {
    let Ok(client) = http() else {
        return Some(String::new());
    };
    let Ok(response) = client
        .get(format!("{}/users.info", s.config.slack.api_base_url))
        .query(&[("user", slack_user)])
        .bearer_auth(token)
        .send()
        .await
    else {
        return Some(String::new());
    };
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if value["ok"] == false
        && value["error"]
            .as_str()
            .is_some_and(|e| INVALID_TOKEN_ERRORS.contains(&e))
    {
        return None;
    }
    let profile = &value["user"]["profile"];
    Some(
        [
            profile["display_name"].as_str(),
            profile["real_name"].as_str(),
            value["user"]["name"].as_str(),
        ]
        .into_iter()
        .flatten()
        .find(|n| !n.trim().is_empty())
        .unwrap_or_default()
        .to_string(),
    )
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
    let already_invalid = body["error"]
        .as_str()
        .is_some_and(|e| INVALID_TOKEN_ERRORS.contains(&e));
    if body["ok"] == true || already_invalid {
        return Ok(());
    }
    Err(failed(
        body["error"].as_str().unwrap_or("unknown").to_string(),
    ))
}

/// 同じ Slack アカウントの連携の保存・解除と、使わないトークンの失効を直列にする。
/// 認可し直しても同じトークンが返るので、ある処理の失効が、並行して保存しようとしている別の処理の
/// トークンまで無効にしうる。行ロックや [`lock_links`] より先に取る
/// (順序: アカウント → 自分の連携の行 → 付け替え → slack_users・投稿)。
async fn lock_account(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_user: &str,
) -> Result<(), AppError> {
    one(
        tx,
        "SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended($1,0)) IS NULL)::text",
        &[&format!("slack-account:{team}:{slack_user}")],
    )
    .await?;
    Ok(())
}

/// 連携できなかったときに受け取ったトークンを片付ける (失敗しても連携の失敗理由を優先する)。
/// 認可し直すと同じトークンが返るので、この Slack アカウントの連携に保存済みなら失効させない
/// (別のワークスペースのアカウントでも同じ)。確かめてから失効させるまでに別の連携が同じトークンを
/// 保存しないよう、アカウントのロックを持ったまま失効させる。判定できなければ失効させない。
async fn discard(s: &AppState, team: &str, slack_user: &str, token: &str) {
    let mut db = s.db.clone();
    let Ok(mut tx) = db.transaction().await else {
        return;
    };
    if lock_account(&mut tx, team, slack_user).await.is_err() {
        return;
    }
    let linked = query(
        &mut tx,
        "SELECT json_build_object('user_id',user_id)::text FROM slack_user_links WHERE team_id=$1 AND slack_user_id=$2",
        &[team, slack_user],
    )
    .await
    .map(|rows| !rows.is_empty())
    .unwrap_or(true);
    if !linked && let Err(e) = revoke(s, token).await {
        tracing::warn!(error = %e, "Failed to revoke an unused Slack token");
    }
    // ロックを確実に放す
    let _ = tx.rollback().await;
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

/// 連携中の Zoo ユーザーが、別の Slack アカウントで連携し直そうとした
const LINKED_OTHER_ACCOUNT: &str = "linked_other_account";

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
    // 別の Slack アカウントへ付け替えると、いまのトークンを失効できないまま失うので、先に解除してもらう
    let current = query(
        tx,
        "SELECT json_build_object('team',team_id,'slack_user',slack_user_id)::text \
         FROM slack_user_links WHERE user_id=$1 FOR UPDATE",
        &[user],
    )
    .await?;
    if current
        .iter()
        .any(|c| c["team"] != team.id.as_str() || c["slack_user"] != slack_user)
    {
        return Err(AppError::Conflict(LINKED_OTHER_ACCOUNT.into()));
    }

    // 上の確認は、まだ行がないと何もロックしない。同じ Zoo ユーザーが別の Slack アカウントで
    // 同時に連携すると、後の INSERT が先の行を上書きしうるので、同じアカウントのときだけ更新する
    let saved = query(
        tx,
        "INSERT INTO slack_user_links(user_id,team_id,team_name,slack_user_id,slack_user_name,access_token_enc,scopes) \
         VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (user_id) DO UPDATE SET \
         team_name=EXCLUDED.team_name,slack_user_name=EXCLUDED.slack_user_name,\
         access_token_enc=EXCLUDED.access_token_enc,scopes=EXCLUDED.scopes,updated_at=now() \
         WHERE slack_user_links.team_id=EXCLUDED.team_id AND slack_user_links.slack_user_id=EXCLUDED.slack_user_id \
         RETURNING json_build_object('user_id',user_id)::text",
        &[user, &team.id, &team.name, slack_user, slack_user_name, token_enc, scopes],
    )
    .await
    .map_err(|e| match e {
        AppError::Conflict(_) => conflict(),
        other => other,
    })?;
    if saved.is_empty() {
        return Err(AppError::Conflict(LINKED_OTHER_ACCOUNT.into()));
    }
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
    // ゲストの参加チャンネルは管理者が決めるため、プレースホルダの参加チャンネルは移さない
    // (single のゲストは 2 つ目のチャンネルに入れず、連携全体が失敗するのも避ける)。
    // is_guest は無効化済み (deactivated) でも立つため、ゲストの種別で判定する
    let guest = one(
        tx,
        "SELECT json_build_object('guest',EXISTS(SELECT 1 FROM users WHERE id=$1 \
         AND guest_kind IN ('single','multi')))::text",
        &[to],
    )
    .await?["guest"]
        == true;
    let mut statements = vec![
        "UPDATE messages SET user_id=$2 WHERE user_id=$1",
        // 本人が同じ絵文字を付けていれば重複を消してから移す
        "DELETE FROM message_reactions r WHERE r.user_id=$1 AND EXISTS(SELECT 1 FROM message_reactions x \
         WHERE x.message_id=r.message_id AND x.user_id=$2 AND x.emoji=r.emoji)",
        "UPDATE message_reactions SET user_id=$2 WHERE user_id=$1",
    ];
    if !guest {
        statements.extend([
            // 新しく参加するチャンネルの過去ログは既読にする。取り込みのワーカーと同じく、チャンネル全体の
            // 最新ではなく同期カーソルまでにして、取り込んだ過去ログより新しい通常の投稿の未読は残す。
            "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) \
             SELECT $2,cu.channel_id,m.latest FROM channel_user cu \
             CROSS JOIN LATERAL (SELECT (to_timestamp(max(sc.last_ts::numeric)) AT TIME ZONE 'UTC')::timestamp(3) \
             AS latest FROM slack_channels sc WHERE sc.channel_id=cu.channel_id AND sc.last_ts IS NOT NULL) m \
             WHERE cu.user_id=$1 AND m.latest IS NOT NULL \
             AND NOT EXISTS(SELECT 1 FROM channel_user mine WHERE mine.user_id=$2 AND mine.channel_id=cu.channel_id) \
             ON CONFLICT (user_id,channel_id) DO NOTHING",
            "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $2,channel_id,joined_at \
             FROM channel_user WHERE user_id=$1 ON CONFLICT DO NOTHING",
            "DELETE FROM channel_user WHERE user_id=$1",
        ]);
    }
    statements.extend([
        "UPDATE attachments SET uploader_id=$2 WHERE uploader_id=$1",
        "UPDATE slack_files SET uploader_id=$2 WHERE uploader_id=$1",
        "UPDATE home_posts SET user_id=$2 WHERE user_id=$1",
        // Home の返信は、返信先の作者を reply_user_id に持つ (本人の Home に他人の返信を出すため)
        "UPDATE home_posts SET reply_user_id=$2 WHERE reply_user_id=$1",
    ]);
    for sql in statements {
        exec(tx, sql, &[from, to]).await?;
    }
    // プレースホルダの users 行は残す (他の表から参照されている可能性があるため)。無効化済みのまま。
    Ok(())
}

/// 取り込んだ本文の `@プレースホルダ` を、本人のユーザー名に書き換える。
/// 取り込みでは `<@U…>` をその時点のユーザー名で本文に保存するため、連携前に取り込んだ過去ログは
/// プレースホルダを指したままになる。`pairs` は (プレースホルダ, 本人) の組で、チームの取り込み先で
/// 書き換えるプレースホルダへのメンションを含む投稿だけをまとめて 1 回でロックして読み、対応表で
/// 置き換える (連携した人ごとに履歴を走査しない)。
async fn rewrite_mentions(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    pairs: &[(String, String)],
) -> Result<(), AppError> {
    if pairs.is_empty() {
        return Ok(());
    }
    let pairs = serde_json::to_string(
        &pairs
            .iter()
            .map(|(from, to)| json!({"f": from, "t": to}))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| AppError::Internal(e.into()))?;
    let names: HashMap<String, String> = query(
        tx,
        "SELECT json_build_object('from',p.user_name,'to',u.user_name)::text \
         FROM json_to_recordset($1::json) AS x(f text,t text) \
         JOIN users p ON p.id=x.f JOIN users u ON u.id=x.t WHERE p.user_name<>u.user_name",
        &[&pairs],
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        Some((
            row["from"].as_str()?.to_string(),
            row["to"].as_str()?.to_string(),
        ))
    })
    .collect();
    if names.is_empty() {
        return Ok(());
    }
    // プレースホルダの名前は slack- で始まるので、まずそれで絞る (安い前段の判定)
    let needle = if names.keys().all(|name| name.starts_with("slack-")) {
        "@slack-"
    } else {
        "@"
    };
    // 書き換える名前へのメンションを含む投稿だけをロックして読む。`@slack-` だけで絞ると、
    // 未連携の人へのメンションを含む投稿までチーム全体でロックし、長いトランザクションになる
    let targets = serde_json::to_string(&names.keys().collect::<Vec<_>>())
        .map_err(|e| AppError::Internal(e.into()))?;
    // 並行して本文を編集した人の変更を上書きしないよう、読む行をロックしてから書き換える
    let rows = query(
        tx,
        "SELECT json_build_object('id',m.id,'content',m.content,'card',m.card)::text FROM messages m \
         WHERE m.channel_id IN (SELECT channel_id FROM slack_channels WHERE team_id=$1) \
         AND strpos(m.content,$2)>0 \
         AND EXISTS(SELECT 1 FROM json_array_elements_text($3::json) AS n(name) \
         WHERE strpos(m.content,'@'||n.name)>0) \
         FOR UPDATE OF m",
        &[team, needle, &targets],
    )
    .await?;
    for row in rows {
        let (Some(id), Some(content)) = (row["id"].as_str(), row["content"].as_str()) else {
            continue;
        };
        let Some(rewritten) = replace_mentions(content, &names) else {
            continue;
        };
        // 書き換えると本文の上限を超える投稿は、切り詰めずにそのまま残す (Home への同期の CHECK にも掛かるため)
        if rewritten.chars().count() > crate::validation::MAX_MESSAGE_CONTENT_LENGTH {
            continue;
        }
        // 本文のカードを書き換えると説明の上限を超えるなら、本文もそのまま残す (表示されるカードとそろえる)
        let card = match row["card"]
            .as_str()
            .map(|card| rewrite_text_card(card, &names))
        {
            Some(Err(())) => continue,
            Some(Ok(card)) => card.unwrap_or_default(),
            None => String::new(),
        };
        exec(
            tx,
            "UPDATE messages SET content=$2,card=COALESCE(NULLIF($3,''),card) WHERE id=$1",
            &[id, &rewritten, &card],
        )
        .await?;
    }
    Ok(())
}

/// カードの説明文の上限 (`MessageCard::encode` と同じ)
const MAX_CARD_DESCRIPTION: usize = 4000;

/// 本文とカードの両方がある投稿は、取り込みで本文を見出し「Slack」のカードとして先頭にも保存している
/// (説明は MFM でなく、リンクを「ラベル (URL)」にした文字列)。カードのある投稿は本文を表示しないので、
/// そのカードの説明のメンションも書き換える。書き換えたときだけ JSON を返す (添付から作ったカードには触れない)。
/// 書き換えると説明の上限を超えるときは `Err` を返す (呼び出し側は投稿全体をそのまま残す)。
fn rewrite_text_card(card: &str, names: &HashMap<String, String>) -> Result<Option<String>, ()> {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(card) else {
        return Ok(None);
    };
    let first = match value.get_mut("cards") {
        Some(cards) => match cards.get_mut(0) {
            Some(first) => first,
            None => return Ok(None),
        },
        None => &mut value,
    };
    if first["title"] != super::worker::IMPORTED_TEXT_CARD_TITLE {
        return Ok(None);
    }
    let Some(rewritten) = first["description"]
        .as_str()
        .and_then(|description| replace_mentions(description, names))
    else {
        return Ok(None);
    };
    if rewritten.chars().count() > MAX_CARD_DESCRIPTION {
        return Err(());
    }
    first["description"] = rewritten.into();
    Ok(Some(value.to_string()))
}

/// `content` の `@name` のうち `names` にあるものを置き換える。変わらなければ `None`。
/// `notifications::extract_mentions` と同じく、名前は英数字と `.`・`-`・`_` で、末尾の `.` は句読点とみなす。
/// `@` の直前は `notifications::is_mention_boundary` と同じ区切りを求める (メールアドレスや URL の途中は対象外)。
fn replace_mentions(content: &str, names: &HashMap<String, String>) -> Option<String> {
    let is_name = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        // 通知のメンション判定と同じ区切り (行頭・空白・開き括弧など) のときだけ。URL の途中の @name は書き換えない
        let boundary = i == 0 || crate::notifications::is_mention_boundary(chars[i - 1]);
        if chars[i] == '@' && boundary {
            let mut end = i + 1;
            while end < chars.len() && is_name(chars[end]) {
                end += 1;
            }
            let token: String = chars[i + 1..end].iter().collect();
            let name = token.trim_end_matches('.');
            if let Some(replacement) = names.get(name) {
                out.push('@');
                out.push_str(replacement);
                out.push_str(&token[name.len()..]);
                changed = true;
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    changed.then_some(out)
}

/// Slack の付け替え (連携の [`reassign`] と [`reconcile_team`]) を直列にする。付け替えは自分の
/// プレースホルダの投稿を UPDATE でロックしてから、取り込み先で書き換えるメンションを含む投稿を
/// まとめてロックするため、重なると互いの投稿を待ってデッドロックする。既存のチャンネルを取り込み先に
/// 選べば、別のワークスペースも同じ Zoo チャンネルに取り込めるので、ワークスペースごとではなく全体で
/// 1 つのロックにする (付け替えは連携時と取り込みの最後にしか走らないので、待ちは問題にならない)。
/// 行ロックを取る前に呼ぶ。
async fn lock_links(tx: &mut toasty::Transaction<'_>) -> Result<(), AppError> {
    one(
        tx,
        "SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended('slack-link',0)) IS NULL)::text",
        &[],
    )
    .await?;
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
    lock_links(tx).await?;
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
    let placeholder = super::user_id(team, slack_user);
    move_content(tx, &placeholder, user).await?;
    rewrite_mentions(tx, team, &[(placeholder, user.to_string())]).await
}

/// 連携済みの Slack ユーザーについて、プレースホルダに残った分を本人へ移す。
/// 取り込みと添付取得の最後に呼び、取り込み中に連携した人の書き込みも回収する。
pub async fn reconcile_team(tx: &mut toasty::Transaction<'_>, team: &str) -> Result<(), AppError> {
    lock_links(tx).await?;
    let linked = query(
        tx,
        "SELECT json_build_object('slack_user_id',slack_user_id,'user_id',user_id)::text \
         FROM slack_users WHERE team_id=$1 AND linked",
        &[team],
    )
    .await?;
    let mut pairs = Vec::with_capacity(linked.len());
    for row in linked {
        let slack_user = row["slack_user_id"].as_str().unwrap_or_default();
        let user = row["user_id"].as_str().unwrap_or_default();
        let placeholder = super::user_id(team, slack_user);
        move_content(tx, &placeholder, user).await?;
        pairs.push((placeholder, user.to_string()));
    }
    rewrite_mentions(tx, team, &pairs).await
}

async fn unlink(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    // complete と同じく、アカウントのロック → 自分の連携の行の順に取る (逆順だとデッドロックする)
    let account = query(
        &mut tx,
        "SELECT json_build_object('team',team_id,'slack_user',slack_user_id)::text FROM slack_user_links WHERE user_id=$1",
        &[&user.id],
    )
    .await?
    .into_iter()
    .next();
    let mut link = None;
    if let Some(account) = account {
        let (team, slack_user) = (
            account["team"].as_str().unwrap_or_default(),
            account["slack_user"].as_str().unwrap_or_default(),
        );
        // 設定を外している間は失効できないので、設定を戻すまで連携 (トークン) を残す
        let key = key(&s)?;
        lock_account(&mut tx, team, slack_user).await?;
        // 待つ間に解除・連携し直された場合は、いまの状態を返す
        link = query(
            &mut tx,
            "SELECT json_build_object('token',access_token_enc,'team_id',team_id,'slack_user_id',slack_user_id)::text \
             FROM slack_user_links WHERE user_id=$1 AND team_id=$2 AND slack_user_id=$3 FOR UPDATE",
            &[&user.id, team, slack_user],
        )
        .await?
        .into_iter()
        .next()
        .map(|link| (key, link));
    }
    if let Some((key, link)) = link {
        // 失効を確かめてから削除する。失敗したらトークンを残し、もう一度解除して再試行できるようにする。
        match decrypt(&key, link["token"].as_str().unwrap_or_default()) {
            Ok(token) => revoke(&s, &token).await?,
            // 鍵を変えて復号できないトークンは Zoo からは失効できない。解除は進め、
            // Slack 側で失効させてもらう (docs/slack-import.md「鍵を変えるとき」)
            Err(_) => tracing::error!(
                user_id = %user.id,
                team_id = link["team_id"].as_str().unwrap_or_default(),
                slack_user_id = link["slack_user_id"].as_str().unwrap_or_default(),
                "Deleting a Slack token that cannot be decrypted; it stays valid until revoked on Slack"
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_mentions_follows_mention_boundaries() {
        let names = HashMap::from([("slack-a.b".to_string(), "alice".to_string())]);
        assert_eq!(
            replace_mentions("@slack-a.b hi (@slack-a.b), @slack-a.b. ok", &names).as_deref(),
            Some("@alice hi (@alice), @alice. ok")
        );
        // 名前の続き・メールアドレス・別の人は書き換えない
        assert_eq!(
            replace_mentions(
                "@slack-a.bc @slack-a.b.c mail@slack-a.b @@slack-a.b",
                &names
            ),
            None
        );
        // URL や Markdown リンクの移動先の @name も書き換えない
        assert_eq!(
            replace_mentions(
                "https://www.npmjs.com/package/@slack-a.b/tool [x](https://e.x/@slack-a.b)",
                &names
            ),
            None
        );
    }
}
