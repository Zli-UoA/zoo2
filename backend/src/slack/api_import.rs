//! 連携した Slack アカウントで、自分が作った非公開チャンネルを取り込む。
//!
//! エクスポートの ZIP は使わず、本人のトークンで Slack の API (`users.conversations`・`conversations.info`・
//! `conversations.members`) を読む。履歴そのものは、差分同期 (`sync.rs`) のエンジンが全履歴から取る。
//! 取り込みを始めるときに、非公開の zoo チャンネルと `slack_channels` の対応を作り、同期カーソル (`last_ts`) を
//! `0` にして同期のキューに入れる (カーソルのないチャンネルは、同期が勝手に全履歴を取らないよう、対象外にしている)。
//!
//! - 取り込めるのは、本人が作った非公開チャンネルだけ (DM・グループ DM は含まない)。Slack の一覧には、本人が
//!   参加しているチャンネルしか出ない。
//! - 新しい zoo チャンネルは常に非公開で作る。作成者とメンバーの扱いは、エクスポートの取り込みと同じ。
//! - 添付ファイルは、差分同期と同じく、ファイル名を本文に残すだけで、取得はしない。

use std::collections::{HashMap, HashSet};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    export::{SlackChannel, SlackUser},
    sync::{self, Slack, SyncError},
    worker::{Context, FilePolicy, create_channel, exec, suppress_app_events},
};
use crate::{apps::query, auth::AuthUser, error::AppError, state::AppState};

/// 一覧・メンバーの取得で辿るページ数の上限 (1 ページ 200 件)。超えたら、不完全な結果を確定せず、エラーにする
const MAX_PAGES: usize = 50;
/// メンバーの名前を `users.info` で調べる人数の上限
const MAX_PROFILE_LOOKUPS: usize = 50;
/// 名前の取得にかけてよい時間
const PROFILE_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/slack/private-channels", get(list))
        .route("/api/slack/private-channels/import", post(start))
        .layer(axum::middleware::map_response(crate::apps::no_store))
}

struct Link {
    team: String,
    slack_user: String,
    token: String,
    scopes: String,
}

impl Link {
    /// 非公開チャンネルの一覧と履歴を読めるスコープがあるか (以前の版で連携した人は、連携し直すまで持たない)
    fn can_read_private(&self) -> bool {
        ["groups:read", "groups:history"]
            .iter()
            .all(|scope| self.scopes.split(',').any(|s| s.trim() == *scope))
    }
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

async fn load_link(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Option<Link>, AppError> {
    Ok(query(
        tx,
        "SELECT json_build_object('team',team_id,'slack_user',slack_user_id,'token',access_token_enc,\
         'scopes',scopes)::text FROM slack_user_links WHERE user_id=$1",
        &[user_id],
    )
    .await?
    .into_iter()
    .next()
    .map(|row| Link {
        team: row["team"].as_str().unwrap_or_default().to_string(),
        slack_user: row["slack_user"].as_str().unwrap_or_default().to_string(),
        token: row["token"].as_str().unwrap_or_default().to_string(),
        scopes: row["scopes"].as_str().unwrap_or_default().to_string(),
    }))
}

fn map_sync(error: SyncError) -> AppError {
    match error {
        SyncError::RateLimited(_) => AppError::RateLimited,
        // トークンが失効している・スコープが足りないなど。本人が連携し直せば直る
        SyncError::NoAccess(reason) => AppError::Forbidden(reason),
        SyncError::InvalidCursor | SyncError::LeaseLost | SyncError::Failed(_) => {
            AppError::BadGateway("Slack request failed".into())
        }
    }
}

/// 一覧を返せない理由 (画面に出す)。`channels` は空
fn unavailable(linked: bool, reason: &str) -> Json<Value> {
    Json(json!({"linked": linked, "reason": reason, "channels": []}))
}

fn valid_slack_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 20
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

async fn list(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let key = key(&s)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let link = load_link(&mut tx, &user.id).await?;
    tx.commit().await?;
    let Some(link) = link else {
        return Ok(unavailable(false, "not_linked"));
    };
    if !s.config.slack.team_allowed(&link.team) {
        return Ok(unavailable(true, "team_mismatch"));
    }
    if !link.can_read_private() {
        return Ok(unavailable(true, "missing_scope"));
    }

    let mut slack = Slack::single(&s, key, &user.id, &link.token)?;
    let mut own: Vec<Value> = Vec::new();
    let mut cursor = String::new();
    for _ in 0..MAX_PAGES {
        let mut params = vec![
            ("types", "private_channel"),
            ("exclude_archived", "false"),
            ("limit", "200"),
        ];
        if !cursor.is_empty() {
            params.push(("cursor", cursor.as_str()));
        }
        let page = match slack.get("users.conversations", &params).await {
            Ok(page) => page,
            Err(SyncError::NoAccess(reason)) => return Ok(unavailable(true, &reason)),
            Err(e) => return Err(map_sync(e)),
        };
        for channel in page["channels"].as_array().into_iter().flatten() {
            if channel["creator"] == link.slack_user.as_str()
                && channel["is_im"] != true
                && channel["is_mpim"] != true
            {
                own.push(channel.clone());
            }
        }
        cursor = page["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if cursor.is_empty() {
            break;
        }
    }
    // 上限に達しても続きがあるなら、一覧を不完全なまま返さない
    if !cursor.is_empty() {
        return Err(AppError::BadGateway("Too many Slack channels".into()));
    }

    let mut tx = db.transaction().await?;
    let mut channels = Vec::with_capacity(own.len());
    for channel in own {
        let id = channel["id"].as_str().unwrap_or_default().to_string();
        let name = channel["name"].as_str().unwrap_or_default().to_string();
        let found = query(
            &mut tx,
            "SELECT json_build_object('channel_id',sc.channel_id,'name',c.channel_name,\
             'member',EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=sc.channel_id AND cu.user_id=$3),\
             'synced',t.last_synced_at IS NOT NULL,'running',COALESCE(t.running,false),\
             'failed',t.last_error IS NOT NULL,'deleted',c.deleted_at IS NOT NULL)::text \
             FROM slack_channels sc JOIN channels c ON c.id=sc.channel_id \
             LEFT JOIN slack_channel_syncs t ON t.team_id=sc.team_id AND t.slack_channel_id=sc.slack_channel_id \
             WHERE sc.team_id=$1 AND sc.slack_channel_id=$2",
            &[&link.team, &id, &user.id],
        )
        .await?
        .into_iter()
        .next();
        // 取り込み先を削除したチャンネルは、過去ログが削除済みのチャンネルに残っていて、取り込み直せないので、一覧に出さない
        // (`start` も、対応が残っている限り 409 を返す)
        if found.as_ref().is_some_and(|row| row["deleted"] == true) {
            continue;
        }
        let mut entry = json!({
            "slack_channel_id": id,
            "name": name,
            "is_archived": channel["is_archived"] == true,
            "members": channel["num_members"].as_i64().unwrap_or(0),
        });
        match found {
            None => {
                // まだ取り込んでいない。名前が使われていれば、`slack-` を付けた名前を提案する
                let taken = !query(
                    &mut tx,
                    "SELECT json_build_object('id',id)::text FROM channels WHERE channel_name=$1",
                    &[&name],
                )
                .await?
                .is_empty();
                let suggested: String = if taken {
                    format!("slack-{name}").chars().take(80).collect()
                } else {
                    name.clone()
                };
                entry["status"] = "new".into();
                entry["name_taken"] = taken.into();
                entry["suggested_name"] = suggested.into();
            }
            Some(row) if row["member"] != true => {
                // 参加していない非公開チャンネルの名前と ID は見せない (チャンネル一覧と同じ)
                entry["status"] = "hidden".into();
            }
            Some(row) => {
                let status = if row["synced"] == true {
                    "done"
                } else if row["failed"] == true {
                    "error"
                } else if row["running"] == true {
                    "running"
                } else {
                    "queued"
                };
                entry["status"] = status.into();
                entry["channel_id"] = row["channel_id"].clone();
                entry["channel_name"] = row["name"].clone();
            }
        }
        channels.push(entry);
    }
    tx.commit().await?;
    Ok(Json(json!({"linked": true, "channels": channels})))
}

/// 1 回の要求で取り込むのは 1 チャンネル (複数を 1 つの要求にすると、途中で失敗したときに一部だけが確定する)
#[derive(Deserialize)]
struct StartRequest {
    slack_channel_id: String,
    name: String,
}

/// `conversations.members` の全ページ
async fn members(slack: &mut Slack, channel: &str) -> Result<Vec<String>, AppError> {
    let mut all = Vec::new();
    let mut cursor = String::new();
    for _ in 0..MAX_PAGES {
        let mut params = vec![("channel", channel), ("limit", "200")];
        if !cursor.is_empty() {
            params.push(("cursor", cursor.as_str()));
        }
        let page = slack
            .get("conversations.members", &params)
            .await
            .map_err(map_sync)?;
        all.extend(
            page["members"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m.as_str().map(str::to_string)),
        );
        cursor = page["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if cursor.is_empty() {
            break;
        }
    }
    // 上限に達しても続きがあるなら、不完全なメンバーで取り込みを確定しない
    if !cursor.is_empty() {
        return Err(AppError::BadGateway(
            "Too many Slack channel members".into(),
        ));
    }
    Ok(all)
}

/// まだ zoo にいないメンバーの名前を `users.info` で調べ、`ctx.profiles` に入れる (調べられなくても止めない)
async fn load_profiles(
    db: &mut toasty::Db,
    slack: &mut Slack,
    ctx: &mut Context,
    members: &[String],
) -> Result<(), AppError> {
    if members.is_empty() {
        return Ok(());
    }
    let mut tx = db.transaction().await?;
    let known: HashSet<String> = query(
        &mut tx,
        "SELECT json_build_object('id',slack_user_id)::text FROM slack_users \
         WHERE team_id=$1 AND slack_user_id IN (SELECT jsonb_array_elements_text($2::jsonb))",
        &[&ctx.team, &json!(members).to_string()],
    )
    .await?
    .into_iter()
    .filter_map(|r| r["id"].as_str().map(str::to_string))
    .collect();
    tx.commit().await?;
    let started = std::time::Instant::now();
    for id in members
        .iter()
        .filter(|id| (id.starts_with('U') || id.starts_with('W')) && !known.contains(*id))
        .take(MAX_PROFILE_LOOKUPS)
    {
        if started.elapsed() >= PROFILE_BUDGET {
            break;
        }
        match slack.get("users.info", &[("user", id.as_str())]).await {
            Ok(value) => {
                if let Ok(user) = serde_json::from_value::<SlackUser>(value["user"].clone()) {
                    ctx.profiles.insert(id.clone(), user);
                }
            }
            Err(SyncError::RateLimited(_)) => break,
            Err(_) => {}
        }
    }
    Ok(())
}

/// 自分が作った非公開チャンネルを取り込む。履歴は、同期のワーカーが全履歴から取る
async fn start(
    State(s): State<AppState>,
    user: AuthUser,
    Json(body): Json<StartRequest>,
) -> Result<Json<Value>, AppError> {
    let key = key(&s)?;
    if !valid_slack_id(&body.slack_channel_id) {
        return Err(AppError::BadRequest("Invalid Slack channel".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let Some(link) = load_link(&mut tx, &user.id).await? else {
        return Err(AppError::BadRequest(
            "Link your Slack account before importing".into(),
        ));
    };
    // SLACK_TEAM_ID を変えたあとに残っている、許可していないワークスペースの連携では取り込まない
    if !s.config.slack.team_allowed(&link.team) {
        return Err(AppError::Forbidden("team_mismatch".into()));
    }
    if !link.can_read_private() {
        return Err(AppError::Forbidden("missing_scope".into()));
    }
    // 通常のチャンネル作成と同じく、ゲストは作れない
    if crate::apps::events::guest(&mut tx, &user.id).await? {
        return Err(AppError::Forbidden(
            "Guests cannot create channels by importing from Slack".into(),
        ));
    }
    tx.commit().await?;

    let mut slack = Slack::single(&s, key, &user.id, &link.token)?;
    let pick = body;
    {
        let info = slack
            .get(
                "conversations.info",
                &[("channel", pick.slack_channel_id.as_str())],
            )
            .await
            .map_err(map_sync)?;
        let channel = &info["channel"];
        // DM・グループ DM と公開チャンネルは取り込まない (Slack の判定を使う)
        if channel["is_private"] != true || channel["is_im"] == true || channel["is_mpim"] == true {
            return Err(AppError::Forbidden(
                "Only private Slack channels can be imported".into(),
            ));
        }
        if channel["creator"] != link.slack_user.as_str() {
            return Err(AppError::Forbidden(
                "You can only import private Slack channels you created".into(),
            ));
        }
        let slack_channel: SlackChannel = serde_json::from_value(json!({
            "id": pick.slack_channel_id,
            "name": channel["name"].as_str().unwrap_or_default(),
            "created": channel["created"].as_i64().unwrap_or(0),
            "creator": link.slack_user,
            "purpose": {"value": channel["purpose"]["value"].as_str().unwrap_or_default()},
            "is_archived": channel["is_archived"] == true,
        }))
        .map_err(|e| AppError::Internal(e.into()))?;

        let member_ids = members(&mut slack, &pick.slack_channel_id).await?;
        let mut ctx = Context {
            team: link.team.clone(),
            users: HashMap::new(),
            profiles: HashMap::new(),
            channel_names: HashMap::new(),
            files: FilePolicy::new(&s),
        };
        load_profiles(&mut db, &mut slack, &mut ctx, &member_ids).await?;

        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        // Slack の応答を待つ間に、単一チャンネルのゲストなどに変わったかもしれない。書き込みと同じトランザクションで確かめ直す
        if crate::apps::events::guest(&mut tx, &user.id).await? {
            return Err(AppError::Forbidden(
                "Guests cannot create channels by importing from Slack".into(),
            ));
        }
        let already = !query(
            &mut tx,
            "SELECT json_build_object('ok',true)::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2",
            &[&link.team, &pick.slack_channel_id],
        )
        .await?
        .is_empty();
        if already {
            return Err(AppError::Conflict(format!(
                "Slack channel {} has already been imported",
                pick.slack_channel_id
            )));
        }
        // 新しい zoo チャンネルは常に非公開で、作成者は本人
        let (target, _) = create_channel(
            &mut tx,
            &link.team,
            &slack_channel,
            pick.name.trim(),
            true,
            &user.id,
        )
        .await?;
        // カーソルを 0 にして同期のキューに入れると、同期が全履歴から取る
        let inserted = query(
            &mut tx,
            "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,$2,$3,'0') \
             ON CONFLICT DO NOTHING RETURNING json_build_object('ok',true)::text",
            &[&link.team, &pick.slack_channel_id, &target.0],
        )
        .await?;
        if inserted.is_empty() {
            return Err(AppError::Conflict(format!(
                "Slack channel {} has already been imported",
                pick.slack_channel_id
            )));
        }
        exec(
            &mut tx,
            "UPDATE channels SET imported_from_slack=true,slack_import_kind=COALESCE(slack_import_kind,'created') WHERE id=$1",
            &[&target.0],
        )
        .await?;
        for member in &member_ids {
            let zoo_user = ctx.user(&mut tx, member, None).await?;
            // 連携したゲストは、管理者が決めたチャンネルにだけ参加する (取り込みでは足さない)。
            // 未連携のプレースホルダは、連携時に本人へ移せるよう参加情報を残す (エクスポートの取り込みと同じ)
            exec(
                &mut tx,
                "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $1,$2,CURRENT_TIMESTAMP \
                 WHERE NOT EXISTS(SELECT 1 FROM users WHERE id=$1 AND guest_kind IN ('single','multi')) \
                 ON CONFLICT DO NOTHING",
                &[&zoo_user.id, &target.0],
            )
            .await?;
        }
        sync::request_channel(&mut tx, &link.team, &pick.slack_channel_id).await?;
        tx.commit().await?;
        Ok(Json(json!({
            "slack_channel_id": pick.slack_channel_id,
            "channel_id": target.0,
            "channel_name": target.1,
        })))
    }
}
