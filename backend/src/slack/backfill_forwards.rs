//! Slack 連携済みチャンネルの過去投稿を Slack Web API から走査し、
//! 転送・引用（`is_share` / `is_msg_unfurl`）の `forwarded_message_ids` 補完と
//! コメントなし転送の取り込み漏れを冪等に修復する 1 回限りのバッチ処理。

use crate::apps::query;
use crate::config::Config;
use crate::error::AppError;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::export::{SlackMessage, compare_ts};
use super::sync::{
    Slack, SyncError, load_channel_tokens, lookup_profiles, parse_messages, threadable,
};
use super::worker::{
    Context, FilePolicy, ensure_plaintext_allowed, id_taken_by_other, insert_message, mapped,
    resolve_forwarded_ids, suppress_app_events,
};

const PAGE_LIMIT: &str = "200";
const MAX_RATE_LIMIT_RETRIES: usize = 3;

#[derive(Debug, Default, Clone)]
pub struct Options {
    pub apply: bool,
    pub channel: Option<String>,
    pub oldest: Option<String>,
    pub max_pages: Option<usize>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub channels: usize,
    pub skipped_channels: usize,
    pub scanned_messages: usize,
    pub matched_backfills: usize,
    pub matched_inserts: usize,
    pub applied_backfills: usize,
    pub applied_inserts: usize,
}

#[derive(Debug)]
struct TargetChannel {
    team_id: String,
    slack_channel_id: String,
    channel_id: String,
    private: bool,
    deleted: bool,
    encrypted: bool,
    token_user_id: Option<String>,
}

pub async fn run(db: &toasty::Db, config: &Config, options: &Options) -> anyhow::Result<Report> {
    let key = config.slack.encryption_key().ok_or_else(|| {
        anyhow::anyhow!("SLACK_TOKEN_ENCRYPTION_KEY is not configured or invalid")
    })?;
    let mut db = db.clone();
    let targets = load_targets(&mut db, options.channel.as_deref().unwrap_or("")).await?;
    let mut report = Report::default();
    let mut team_channel_names: HashMap<String, HashMap<String, String>> = HashMap::new();

    for target in targets {
        if target.deleted || target.encrypted || !config.slack.team_allowed(&target.team_id) {
            report.skipped_channels += 1;
            continue;
        }
        let tokens = load_channel_tokens(
            &mut db,
            &target.team_id,
            &target.channel_id,
            target.private,
            target.token_user_id.as_deref(),
        )
        .await?;
        if tokens.is_empty() {
            tracing::info!(
                team = %target.team_id,
                channel = %target.slack_channel_id,
                "Skipping channel without linked Slack user tokens"
            );
            report.skipped_channels += 1;
            continue;
        }
        let mut slack = Slack::from_tokens(config.slack.api_base_url.clone(), key, tokens)?;
        let info = match slack_get_with_retry(
            &mut slack,
            "conversations.info",
            &[("channel", target.slack_channel_id.as_str())],
        )
        .await
        {
            Ok(info) => info,
            Err(SyncError::NoAccess(err) | SyncError::Failed(err)) => {
                tracing::warn!(
                    team = %target.team_id,
                    channel = %target.slack_channel_id,
                    error = %err,
                    "Skipping unreadable Slack channel during backfill"
                );
                report.skipped_channels += 1;
                continue;
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "conversations.info failed for {}: {}",
                    target.slack_channel_id,
                    sync_error_message(&err)
                ));
            }
        };
        let slack_private = ["is_private", "is_im", "is_mpim", "is_group"]
            .iter()
            .any(|k| info["channel"][*k] == true);
        if slack_private && !target.private {
            tracing::warn!(
                team = %target.team_id,
                channel = %target.slack_channel_id,
                "Skipping private Slack channel mapped to public zoo channel"
            );
            report.skipped_channels += 1;
            continue;
        }

        if !team_channel_names.contains_key(&target.team_id) {
            let names = load_team_channel_names(&mut db, &target.team_id).await?;
            team_channel_names.insert(target.team_id.clone(), names);
        }
        let mut ctx = Context {
            team: target.team_id.clone(),
            users: HashMap::new(),
            profiles: HashMap::new(),
            channel_names: team_channel_names
                .get(&target.team_id)
                .cloned()
                .unwrap_or_default(),
            files: FilePolicy {
                token: config.slack.import_bot_token.is_some(),
                max_size: config.s3.max_file_size,
                base_url: config.slack.files_base_url.clone(),
            },
            app_id: config.slack.app_id.clone(),
        };
        // 差分同期と同様にここでは添付ファイルを同期ダウンロードしない（orphan_files_worker に任せる）
        ctx.files.token = false;

        let oldest = options.oldest.as_deref().unwrap_or("0");
        let mut pages = 0usize;
        let mut cursor: Option<String> = None;
        let mut thread_roots: Vec<String> = Vec::new();
        let mut seen_threads: HashSet<String> = HashSet::new();

        loop {
            if options.max_pages.is_some_and(|max| pages >= max) {
                break;
            }
            let mut params = vec![
                ("channel", target.slack_channel_id.as_str()),
                ("oldest", oldest),
                ("limit", PAGE_LIMIT),
            ];
            if let Some(cur) = cursor.as_deref() {
                params.push(("cursor", cur));
            }
            let page = slack_get_with_retry(&mut slack, "conversations.history", &params)
                .await
                .map_err(|err| anyhow::anyhow!("{}", sync_error_message(&err)))?;
            pages += 1;
            let messages = parse_messages(&page);
            report.scanned_messages += messages.len();
            for m in &messages {
                let top_level = m.thread_ts.as_deref().is_none_or(|t| t == m.ts.as_str());
                if top_level
                    && m.reply_count.unwrap_or(0) > 0
                    && threadable(m)
                    && seen_threads.insert(m.ts.clone())
                {
                    thread_roots.push(m.ts.clone());
                }
            }
            process_messages(
                &mut db,
                &mut slack,
                &mut ctx,
                &target,
                &messages,
                options.apply,
                &mut report,
            )
            .await?;

            let next = page["response_metadata"]["next_cursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if page["has_more"] == true && next.is_some() {
                cursor = next;
            } else {
                break;
            }
        }

        for thread_ts in thread_roots {
            if options.max_pages.is_some_and(|max| pages >= max) {
                break;
            }
            let mut tcursor: Option<String> = None;
            loop {
                if options.max_pages.is_some_and(|max| pages >= max) {
                    break;
                }
                let mut params = vec![
                    ("channel", target.slack_channel_id.as_str()),
                    ("ts", thread_ts.as_str()),
                    ("oldest", oldest),
                    ("limit", PAGE_LIMIT),
                ];
                if let Some(cur) = tcursor.as_deref() {
                    params.push(("cursor", cur));
                }
                let page = match slack_get_with_retry(&mut slack, "conversations.replies", &params)
                    .await
                {
                    Ok(page) => page,
                    Err(SyncError::Failed(err)) if err.ends_with("thread_not_found") => break,
                    Err(err) => return Err(anyhow::anyhow!("{}", sync_error_message(&err))),
                };
                pages += 1;
                let messages = parse_messages(&page);
                // 先頭要素（スレッド親）は履歴走査で処理済みのことが多いが、重複処理も冪等に抑止される
                let replies: Vec<SlackMessage> =
                    messages.into_iter().filter(|m| m.ts != thread_ts).collect();
                report.scanned_messages += replies.len();
                process_messages(
                    &mut db,
                    &mut slack,
                    &mut ctx,
                    &target,
                    &replies,
                    options.apply,
                    &mut report,
                )
                .await?;

                let next = page["response_metadata"]["next_cursor"]
                    .as_str()
                    .filter(|c| !c.is_empty())
                    .map(str::to_string);
                if page["has_more"] == true && next.is_some() {
                    tcursor = next;
                } else {
                    break;
                }
            }
        }

        report.channels += 1;
    }

    Ok(report)
}

async fn process_messages(
    db: &mut toasty::Db,
    slack: &mut Slack,
    ctx: &mut Context,
    target: &TargetChannel,
    messages: &[SlackMessage],
    apply: bool,
    report: &mut Report,
) -> anyhow::Result<()> {
    let mut with_forwards: Vec<&SlackMessage> = messages
        .iter()
        .filter(|m| {
            m.is_importable()
                && !(ctx.app_id.is_some() && m.app_id == ctx.app_id)
                && !m.forwarded_refs(Some(&target.team_id)).is_empty()
        })
        .collect();
    if with_forwards.is_empty() {
        return Ok(());
    }
    with_forwards.sort_by(|a, b| compare_ts(&a.ts, &b.ts));

    if apply {
        let owned: Vec<SlackMessage> = with_forwards.iter().map(|m| (*m).clone()).collect();
        lookup_profiles(db, slack, ctx, &owned)
            .await
            .map_err(|err| anyhow::anyhow!("{}", sync_error_message(&err)))?;
    }

    let mut tx = db.transaction().await.map_err(AppError::from)?;
    if apply {
        suppress_app_events(&mut tx).await?;
        ensure_plaintext_allowed(&mut tx, &target.channel_id).await?;
    }

    for message in with_forwards {
        let own = mapped(
            &mut tx,
            &target.team_id,
            &target.slack_channel_id,
            &message.ts,
        )
        .await?;
        if own.as_ref().is_some_and(|(_, origin)| origin == "zoo") {
            continue;
        }
        let derived = own.is_none();
        let id = own.map(|(id, _)| id).unwrap_or_else(|| {
            super::message_id(&target.team_id, &target.slack_channel_id, &message.ts)
        });
        if derived && id_taken_by_other(&mut tx, &id, &target.channel_id, &message.ts).await? {
            continue;
        }
        let forwarded_ids = resolve_forwarded_ids(&mut tx, &target.team_id, message).await?;
        if forwarded_ids.is_empty() {
            continue;
        }
        let ts_guard = if derived { message.ts.as_str() } else { "" };
        let existing = query(
            &mut tx,
            "SELECT json_build_object('deleted',deleted_at IS NOT NULL,'has_forwards',forwarded_message_ids IS NOT NULL)::text \
             FROM messages WHERE id=$1 AND channel_id=$2 \
             AND ($3='' OR created_at=(to_timestamp($3::numeric) AT TIME ZONE 'UTC')::timestamp(3))",
            &[&id, &target.channel_id, ts_guard],
        )
        .await?;

        match existing.first() {
            Some(row) => {
                if row["deleted"] == true || row["has_forwards"] == true {
                    continue;
                }
                report.matched_backfills += 1;
                if apply {
                    let forwarded_json = serde_json::to_string(&forwarded_ids)?;
                    let updated = query(
                        &mut tx,
                        "UPDATE messages SET forwarded_message_ids=$2::jsonb \
                         WHERE id=$1 AND channel_id=$3 AND deleted_at IS NULL \
                         AND forwarded_message_ids IS NULL \
                         AND ($4='' OR created_at=(to_timestamp($4::numeric) AT TIME ZONE 'UTC')::timestamp(3)) \
                         RETURNING json_build_object('ok',true)::text",
                        &[&id, &forwarded_json, &target.channel_id, ts_guard],
                    )
                    .await?;
                    if !updated.is_empty() {
                        report.applied_backfills += 1;
                    }
                }
            }
            None => {
                report.matched_inserts += 1;
                if apply {
                    insert_message(
                        &mut tx,
                        ctx,
                        &target.slack_channel_id,
                        &target.channel_id,
                        "",
                        false,
                        message,
                    )
                    .await?;
                    let inserted = query(
                        &mut tx,
                        "SELECT json_build_object('ok',true)::text \
                         FROM messages WHERE id=$1 AND channel_id=$2 AND forwarded_message_ids IS NOT NULL",
                        &[&id, &target.channel_id],
                    )
                    .await?;
                    if !inserted.is_empty() {
                        report.applied_inserts += 1;
                    }
                }
            }
        }
    }

    if apply {
        tx.commit().await.map_err(AppError::from)?;
    } else {
        let _ = tx.rollback().await;
    }
    Ok(())
}

async fn load_targets(
    db: &mut toasty::Db,
    channel_filter: &str,
) -> Result<Vec<TargetChannel>, AppError> {
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        "SELECT json_build_object(\
           'team_id', sc.team_id,\
           'slack_channel_id', sc.slack_channel_id,\
           'channel_id', sc.channel_id,\
           'private', c.is_private,\
           'deleted', c.deleted_at IS NOT NULL,\
           'encrypted', c.encrypted,\
           'token_user_id', sync.token_user_id\
         )::text \
         FROM slack_channels sc \
         JOIN channels c ON c.id = sc.channel_id \
         LEFT JOIN slack_channel_syncs sync \
           ON sync.team_id = sc.team_id AND sync.slack_channel_id = sc.slack_channel_id \
         WHERE ($1 = '' OR sc.channel_id = $1 OR sc.slack_channel_id = $1) \
         ORDER BY sc.team_id, sc.slack_channel_id",
        &[channel_filter],
    )
    .await?;
    let _ = tx.rollback().await;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let (Some(team_id), Some(slack_channel_id), Some(channel_id)) = (
            row["team_id"].as_str(),
            row["slack_channel_id"].as_str(),
            row["channel_id"].as_str(),
        ) else {
            continue;
        };
        out.push(TargetChannel {
            team_id: team_id.to_string(),
            slack_channel_id: slack_channel_id.to_string(),
            channel_id: channel_id.to_string(),
            private: row["private"] == true,
            deleted: row["deleted"] == true,
            encrypted: row["encrypted"] == true,
            token_user_id: row["token_user_id"].as_str().map(str::to_string),
        });
    }
    Ok(out)
}

async fn load_team_channel_names(
    db: &mut toasty::Db,
    team_id: &str,
) -> Result<HashMap<String, String>, AppError> {
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        "SELECT json_build_object('slack',s.slack_channel_id,'name',c.channel_name)::text \
         FROM slack_channels s JOIN channels c ON c.id=s.channel_id \
         WHERE s.team_id=$1 AND c.deleted_at IS NULL AND NOT c.is_private",
        &[team_id],
    )
    .await?;
    let _ = tx.rollback().await;
    let mut names = HashMap::new();
    for row in rows {
        if let (Some(slack), Some(name)) = (row["slack"].as_str(), row["name"].as_str()) {
            names.insert(slack.to_string(), name.to_string());
        }
    }
    Ok(names)
}

async fn slack_get_with_retry(
    slack: &mut Slack,
    method: &str,
    params: &[(&str, &str)],
) -> Result<serde_json::Value, SyncError> {
    let mut retries = 0usize;
    loop {
        match slack.get(method, params).await {
            Ok(val) => return Ok(val),
            Err(SyncError::RateLimited(secs)) if retries < MAX_RATE_LIMIT_RETRIES => {
                retries += 1;
                tokio::time::sleep(Duration::from_secs(secs.clamp(1, 300))).await;
            }
            Err(err) => return Err(err),
        }
    }
}

fn sync_error_message(err: &SyncError) -> String {
    match err {
        SyncError::RateLimited(secs) => format!("rate limited (retry_after={secs}s)"),
        SyncError::NoAccess(e) => format!("no access: {e}"),
        SyncError::InvalidCursor => "invalid cursor".into(),
        SyncError::LeaseLost => "lease lost".into(),
        SyncError::Failed(e) => e.clone(),
    }
}
