//! 各ユーザーが、連携した Slack アカウントで作ったチャンネルを、管理者が保存したエクスポートから取り込む (#291)。
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
use crate::{apps::query, auth::AuthUser, error::AppError, state::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/slack/imports", get(list).post(start))
        .layer(axum::middleware::map_response(crate::apps::no_store))
}

/// 連携している Slack アカウントの (ワークスペース, Slack ユーザー)
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

/// 自分が作った Slack チャンネルと、それぞれに選べる既存の取り込み先 (`targets`)
async fn own_channels(
    tx: &mut toasty::Transaction<'_>,
    export: &Value,
    slack_user: &str,
    user_id: &str,
) -> Result<Vec<Value>, AppError> {
    let mut channels = Vec::new();
    for channel in export["analysis"]["channels"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if channel["creator"] != slack_user || channel["deleted_channel"].is_string() {
            continue;
        }
        let private = channel["is_private"] == true;
        let targets = if channel["existing_channel"].is_object() {
            // 以前に取り込んだチャンネルは、その取り込み先にしか入れられない
            vec![channel["existing_channel"].clone()]
        } else if let Some(same) = channel["same_name_channel"].as_object() {
            // 他人のチャンネルに勝手に履歴を足さないよう、自分が参加しているものだけを選べる。
            // 非公開チャンネルの履歴は、公開チャンネルには入れない。
            let joined = !query(
                tx,
                "SELECT json_build_object('id',channel_id)::text FROM channel_user \
                 WHERE channel_id=$1 AND user_id=$2",
                &[same["id"].as_str().unwrap_or_default(), user_id],
            )
            .await?
            .is_empty();
            if joined && (!private || same["is_private"] == true) {
                vec![Value::Object(same.clone())]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let mut entry = channel.clone();
        entry["targets"] = Value::Array(targets);
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
        && let Some(id) = latest_export(&mut tx, &team).await?
    {
        let job = load_job(&mut tx, &id).await?;
        channels = own_channels(&mut tx, &job, &slack_user, &user.id).await?;
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
    let Some(id) = latest_export(&mut tx, &team).await? else {
        return Err(AppError::Conflict(
            "No Slack export is available to import".into(),
        ));
    };
    let export = lock_export(&mut tx, &id).await?;
    let channels = own_channels(&mut tx, &export, &slack_user, &user.id).await?;
    let own: HashMap<&str, &Value> = channels
        .iter()
        .filter_map(|c| Some((c["slack_channel_id"].as_str()?, c)))
        .collect();
    let mut mappings = Vec::with_capacity(body.mappings.len());
    for mapping in body.mappings {
        let Some(channel) = own.get(mapping.slack_channel_id.as_str()) else {
            return Err(AppError::Forbidden(format!(
                "You can only import Slack channels you created ({})",
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
                        "Slack channel {} can only be imported into a channel with the same name that you have joined",
                        mapping.slack_channel_id
                    )));
                }
                MappingAction::Existing { channel_id }
            }
            // 公開・非公開は Slack に合わせる
            MappingAction::Create { name, .. } => MappingAction::Create {
                name,
                is_private: Some(channel["is_private"] == true),
            },
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
