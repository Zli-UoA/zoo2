//! 各ユーザーが、連携した Slack アカウントで作った非公開チャンネルを、管理者が保存したエクスポートから取り込む (#291)。
//! 公開チャンネルの取り込みは管理者だけが行う (`import.rs`)。DM・グループ DM は取り込まない。
//!
//! 取り込み先は、Slack と同じ名前の既存チャンネル (自分が参加しているもの) か、新しく作るチャンネルから選ぶ。
//! 以前に取り込んだチャンネルは、同じ取り込み先に固定する (`import::ensure_same_target`)。
//! 取り込み自体は、管理者の取り込みと同じジョブ (`import::create_run`) でワーカーが行う。

use std::collections::HashMap;

use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use super::import::{
    JOB_JSON, Mapping, MappingAction, StartRequest, create_run, load_job, lock_export,
    validate_mappings,
};
use crate::{
    apps::query,
    auth::{AuthUser, rbac::is_workspace_admin},
    error::AppError,
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/slack/imports", get(list).post(start))
        .layer(axum::middleware::map_response(crate::apps::no_store))
}

/// 連携している Slack アカウントの (ワークスペース, Slack ユーザー)。
/// `SLACK_TEAM_ID` で許可しているかは、呼び出し側で `SlackConfig::team_allowed` を確かめる
async fn linked(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Option<(String, String)>, AppError> {
    Ok(query(
        tx,
        "SELECT json_build_object('team',team_id,'slack_user',slack_user_id)::text \
         FROM slack_user_links WHERE user_id=$1",
        &[user_id],
    )
    .await?
    .into_iter()
    .next()
    .map(|row| {
        (
            row["team"].as_str().unwrap_or_default().to_string(),
            row["slack_user"].as_str().unwrap_or_default().to_string(),
        )
    }))
}

/// そのワークスペースの、取り込みに使える最新のエクスポート
async fn latest_export(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
) -> Result<Option<String>, AppError> {
    Ok(query(
        tx,
        "SELECT json_build_object('id',id)::text FROM slack_import_jobs \
         WHERE source_id IS NULL AND status='ready' AND team_id=$1 \
         ORDER BY created_at DESC,id DESC LIMIT 1",
        &[team],
    )
    .await?
    .into_iter()
    .next()
    .and_then(|row| row["id"].as_str().map(str::to_string)))
}

/// `channel_id` に、取り込みで履歴とメンバーを足してよいか。
/// 取り込みは ZIP のメンバーも `channel_user` に足すので、参加していることに加えて、
/// 通常のメンバー追加と同じ権限 (`rbac::can_add_member`) と、通常の投稿と同じ投稿権限を求める。
pub(crate) async fn can_import_into(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    user_id: &str,
    admin: bool,
    slack_private: bool,
) -> Result<bool, AppError> {
    let Some(row) = query(
        tx,
        "SELECT json_build_object('creator',c.created_user_id,'private',c.is_private,'dm',c.is_dm,\
         'policy',c.posting_policy,\
         'joined',EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=$2))::text \
         FROM channels c WHERE c.id=$1 AND c.deleted_at IS NULL AND c.archive_at IS NULL",
        &[channel_id, user_id],
    )
    .await?
    .into_iter()
    .next() else {
        return Ok(false);
    };
    let private = row["private"] == true;
    let joined = row["joined"] == true;
    // 投稿を管理者に限ったチャンネルには、通常の投稿 (`apps::events::posting`) と同じく管理者しか書き込めない
    let can_post = row["policy"] != "managers" || row["creator"] == user_id || admin;
    // 非公開チャンネルの履歴は、公開チャンネルには入れない
    Ok(joined
        && can_post
        && (!slack_private || private)
        && crate::auth::rbac::can_add_member(
            private,
            row["creator"].as_str().unwrap_or_default(),
            user_id,
            joined,
            admin,
            row["dm"] == true,
        ))
}

async fn joined(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    user_id: &str,
) -> Result<bool, AppError> {
    Ok(!query(
        tx,
        "SELECT json_build_object('ok',true)::text FROM channel_user WHERE channel_id=$1 AND user_id=$2",
        &[channel_id, user_id],
    )
    .await?
    .is_empty())
}

/// 自分が作った Slack チャンネルと、それぞれに選べる既存の取り込み先 (`targets`)
async fn own_channels(
    tx: &mut toasty::Transaction<'_>,
    export: &Value,
    slack_user: &str,
    user_id: &str,
    admin: bool,
) -> Result<Vec<Value>, AppError> {
    let mut channels = Vec::new();
    let guest = crate::apps::events::guest(tx, user_id).await?;
    for channel in export["analysis"]["channels"]
        .as_array()
        .into_iter()
        .flatten()
    {
        // 公開チャンネルの取り込みは管理者だけ。非公開チャンネル (`groups.json` 由来) だけを本人に任せる
        if channel["creator"] != slack_user
            || channel["is_private"] != true
            || channel["deleted_channel"].is_string()
        {
            continue;
        }
        // 以前に取り込んだチャンネルは、その取り込み先にしか入れられない。
        // それ以外は、他人のチャンネルに勝手に履歴を足さないよう、同名で自分が参加しているものだけを選べる。
        // どちらも、現在の参加状況とメンバー追加の権限を確かめる。
        let candidate = channel["existing_channel"]
            .as_object()
            .or_else(|| channel["same_name_channel"].as_object());
        let mut targets = Vec::new();
        if let Some(target) = candidate
            && can_import_into(
                tx,
                target["id"].as_str().unwrap_or_default(),
                user_id,
                admin,
                channel["is_private"] == true,
            )
            .await?
        {
            targets.push(Value::Object(target.clone()));
        }
        let mut entry = channel.clone();
        entry["targets"] = Value::Array(targets);
        // 参加していない非公開チャンネル (ゲストには公開チャンネルも) の名前と ID は、
        // チャンネル一覧 (`list_channels`) と同じく見せない。以前の取り込み先は、そこにしか取り込めないので
        // 「ある」ことだけを `existing_hidden` で返す (同じ名前のチャンネルがあることは `name_taken` で分かる)
        for key in ["existing_channel", "same_name_channel"] {
            let id = entry[key]["id"].as_str().unwrap_or_default().to_string();
            if !entry[key].is_object() {
                continue;
            }
            let restricted = entry[key]["is_private"] == true || guest;
            if restricted && !joined(tx, &id, user_id).await? {
                entry[key] = Value::Null;
                if key == "existing_channel" {
                    entry["existing_hidden"] = true.into();
                }
            }
        }
        channels.push(entry);
    }
    Ok(channels)
}

async fn list(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let mut export = Value::Null;
    let mut channels = Vec::new();
    if let Some((team, slack_user)) = linked(&mut tx, &user.id).await?
        && s.config.slack.team_allowed(&team)
        && let Some(id) = latest_export(&mut tx, &team).await?
    {
        let job = load_job(&mut tx, &id).await?;
        let admin = is_workspace_admin(&mut tx, &user.id, s.config.dev_auth_enabled).await?;
        channels = own_channels(&mut tx, &job, &slack_user, &user.id, admin).await?;
        export = json!({
            "id": job["id"],
            "file_name": job["file_name"],
            "created_at": job["created_at"],
        });
    }
    let imports = query(
        &mut tx,
        &format!(
            "SELECT {JOB_JSON} FROM slack_import_jobs j \
             WHERE j.created_by=$1 AND j.source_id IS NOT NULL ORDER BY j.created_at DESC LIMIT 5"
        ),
        &[&user.id],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "export": export,
        "channels": channels,
        "imports": imports,
    })))
}

/// 自分が作ったチャンネルを取り込む。作るチャンネルの作成者・メンバーには本人が入る。
async fn start(
    State(s): State<AppState>,
    user: AuthUser,
    Json(body): Json<StartRequest>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let Some((team, slack_user)) = linked(&mut tx, &user.id).await? else {
        return Err(AppError::BadRequest(
            "Link your Slack account before importing".into(),
        ));
    };
    // SLACK_TEAM_ID を変えたあとに残っている、許可していないワークスペースの連携では取り込まない
    if !s.config.slack.team_allowed(&team) {
        return Err(AppError::Forbidden("team_mismatch".into()));
    }
    let Some(id) = latest_export(&mut tx, &team).await? else {
        return Err(AppError::Conflict(
            "No Slack export is available to import".into(),
        ));
    };
    let export = lock_export(&mut tx, &id).await?;
    let admin = is_workspace_admin(&mut tx, &user.id, s.config.dev_auth_enabled).await?;
    let channels = own_channels(&mut tx, &export, &slack_user, &user.id, admin).await?;
    // 通常のチャンネル作成 (`routes::channels::create_channel`) と同じく、ゲストは作れない
    let creates = body
        .mappings
        .iter()
        .any(|m| matches!(m.action, MappingAction::Create { .. }));
    if creates && crate::apps::events::guest(&mut tx, &user.id).await? {
        return Err(AppError::Forbidden(
            "Guests cannot create channels by importing from Slack".into(),
        ));
    }
    let own: HashMap<&str, &Value> = channels
        .iter()
        .filter_map(|c| Some((c["slack_channel_id"].as_str()?, c)))
        .collect();
    let mut mappings = Vec::with_capacity(body.mappings.len());
    for mapping in body.mappings {
        let Some(channel) = own.get(mapping.slack_channel_id.as_str()) else {
            return Err(AppError::Forbidden(format!(
                "You can only import private Slack channels you created; public channels are imported by workspace admins ({})",
                mapping.slack_channel_id
            )));
        };
        let action = match mapping.action {
            MappingAction::Existing { channel_id } => {
                let allowed = channel["targets"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|t| t["id"] == channel_id.as_str()));
                if !allowed {
                    return Err(AppError::Forbidden(format!(
                        "Slack channel {} can only be imported into a channel with the same name that you have joined and can add members to",
                        mapping.slack_channel_id
                    )));
                }
                MappingAction::Existing { channel_id }
            }
            MappingAction::Create { name, .. } => {
                // 以前の取り込み先が見えないチャンネルなら、そこにしか取り込めないので断る。
                // 取り込み先の検証 (`ensure_same_target`) のエラーはチャンネル名を含むので、その前に名前を出さずに返す
                if channel["existing_hidden"] == true {
                    return Err(AppError::Forbidden(format!(
                        "Slack channel {} was already imported into a channel you cannot see",
                        mapping.slack_channel_id
                    )));
                }
                // 以前の取り込みで作ったチャンネルがあれば、ワーカーはそこへ入れて本人と ZIP のメンバーを足す。
                // その場合は既存チャンネルへの取り込みと同じく、参加状況とメンバー追加の権限を確かめる
                let own_id = super::channel_id(&team, &mapping.slack_channel_id);
                let exists = !query(
                    &mut tx,
                    "SELECT json_build_object('id',id)::text FROM channels WHERE id=$1",
                    &[&own_id],
                )
                .await?
                .is_empty();
                // 前回の取り込みで作ったものではない同じ ID のチャンネルは、他人が先に作った可能性があるので再利用しない
                // (名前は出さない)。ワーカーも同じ確認をする
                if exists
                    && query(
                        &mut tx,
                        "SELECT json_build_object('ok',true)::text FROM slack_channels \
                         WHERE team_id=$1 AND slack_channel_id=$2 AND channel_id=$3",
                        &[&team, &mapping.slack_channel_id, &own_id],
                    )
                    .await?
                    .is_empty()
                {
                    return Err(AppError::Conflict(format!(
                        "Slack channel {} cannot be imported into a new channel because a channel with the same ID already exists",
                        mapping.slack_channel_id
                    )));
                }
                if exists
                    && !can_import_into(
                        &mut tx,
                        &own_id,
                        &user.id,
                        admin,
                        channel["is_private"] == true,
                    )
                    .await?
                {
                    return Err(AppError::Forbidden(format!(
                        "Slack channel {} was already imported into a channel you cannot add members to",
                        mapping.slack_channel_id
                    )));
                }
                // 非公開チャンネルだけがここに来るので、作るチャンネルも非公開になる
                MappingAction::Create {
                    name,
                    is_private: Some(channel["is_private"] == true),
                    // 自分のチャンネルの取り込みでは、アーカイブの指定はできない
                    archive: false,
                }
            }
            MappingAction::Skip => MappingAction::Skip,
        };
        mappings.push(Mapping {
            slack_channel_id: mapping.slack_channel_id,
            action,
        });
    }
    let total = validate_mappings(&mut tx, &export, &mappings).await?;
    let run = create_run(&mut tx, &export, &user.id, &mappings, total).await?;
    tx.commit().await?;
    Ok(Json(run))
}
