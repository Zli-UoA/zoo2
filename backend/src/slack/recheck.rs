//! 直近 24 時間の履歴の再確認 (#293)。イベントの取りこぼし (再送の上限を超えた、受信口の停止中) の保険。
//!
//! 同期は、新しい投稿しか取らず、取り込み済みの投稿を取り直さない。イベントを取りこぼすと、編集や削除は、ポーリングだけでは
//! 反映されない。そこで、30 分に 1 回、直近 24 時間の履歴を取り直して、Zoo と突き合わせる (イベントと同じく、API の
//! 状態に Zoo を合わせる)。
//!
//! - **編集**: すでに Zoo にある投稿に、`hints.rs` と同じ規則 (`edit`) で反映する。リアクションの追加も入れる。
//!   リアクションの削除は、反映しない (Zoo で付けたリアクションを、Slack に届く前に消さないため)。
//! - **削除**: `tombstone` は削除。さらに、差分同期・イベントで取り込んだ投稿 (`slack_message_map` の `slack`。
//!   取り込みの時に、Slack でスレッドの返信でない投稿だけを残し、3 日で掃除する) が、履歴から消えていれば削除する。
//!   返信は、履歴に出ないので対象にしない (親を Zoo で結び付けられなかった返信も同じ)。
//!   Zoo で作った投稿や、ZIP で取り込んだ投稿は、対応表にないので、消えたとは見なさない。
//! - **安全のための条件**: 履歴を最後まで (5 ページ・1000 件まで) 取れたときだけ、「消えた」を判定する。履歴が不完全な
//!   (空や一部だけが返る) ときの大量の削除を防ぐため、候補の半数を超え、かつ 3 件以上が消えたように見えるときは、
//!   削除しない (ログに残す)。
//! - **API の呼び出し**: 履歴のページだけ (1 チャンネル 1 回あたり最大 5 回)。スレッドの返信は見ない。

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::{
    export::SlackMessage,
    hints::{edit, exec_delete},
    sync::{Slack, SyncError, lookup_profiles, parse_messages},
    worker::{Context, insert_reactions, suppress_app_events},
};
use crate::{apps::query, error::AppError};

/// 突き合わせる期間
const WINDOW_SECS: u64 = 24 * 60 * 60;
const MAX_PAGES: usize = 5;
/// 1 回のトランザクションで反映する投稿の数
const CHUNK: usize = 100;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(super) async fn run(
    slack: &mut Slack,
    ctx: &mut Context,
    team: &str,
    slack_channel: &str,
    zoo_channel: &str,
    lease_token: &str,
    db: &mut toasty::Db,
) -> Result<(), SyncError> {
    let oldest = format!("{}.000000", now_secs().saturating_sub(WINDOW_SECS));
    let mut messages: Vec<SlackMessage> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut complete = false;
    for _ in 0..MAX_PAGES {
        let mut params = vec![
            ("channel", slack_channel),
            ("oldest", oldest.as_str()),
            ("limit", "200"),
        ];
        if let Some(cursor) = cursor.as_deref() {
            params.push(("cursor", cursor));
        }
        let page = slack.get("conversations.history", &params).await?;
        messages.extend(parse_messages(&page));
        let next = page["response_metadata"]["next_cursor"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        if page["has_more"] == true && next.is_some() {
            cursor = next;
        } else {
            complete = true;
            break;
        }
    }
    let importable: Vec<SlackMessage> = messages
        .iter()
        .filter(|m| m.is_importable())
        .cloned()
        .collect();
    lookup_profiles(db, slack, ctx, &importable).await?;

    for chunk in messages.chunks(CHUNK) {
        apply_chunk(
            ctx,
            team,
            slack_channel,
            zoo_channel,
            lease_token,
            chunk,
            db,
        )
        .await?;
    }

    let mut tx = db.transaction().await.map_err(AppError::from)?;
    suppress_app_events(&mut tx).await?;
    if complete {
        let present: HashSet<&str> = messages.iter().map(|m| m.ts.as_str()).collect();
        let candidates = query(
            &mut tx,
            "SELECT json_build_object('ts',m.ts,'id',m.message_id)::text FROM slack_message_map m \
             JOIN messages x ON x.id=m.message_id \
             WHERE m.team_id=$1 AND m.slack_channel_id=$2 AND m.origin='slack' AND m.ts::numeric>$3::numeric \
             AND x.channel_id=$4 AND x.parent_id IS NULL AND x.deleted_at IS NULL",
            &[team, slack_channel, &oldest, zoo_channel],
        )
        .await?;
        let missing: Vec<&Value> = candidates
            .iter()
            .filter(|c| c["ts"].as_str().is_some_and(|ts| !present.contains(ts)))
            .collect();
        if missing.len() >= 3 && missing.len() * 2 > candidates.len() {
            tracing::warn!(
                team,
                channel = slack_channel,
                missing = missing.len(),
                candidates = candidates.len(),
                "Slack recheck found too many missing posts; not deleting"
            );
        } else {
            for row in missing {
                if let Some(id) = row["id"].as_str() {
                    exec_delete(&mut tx, id, zoo_channel).await?;
                }
            }
        }
    }
    let done = query(
        &mut tx,
        "UPDATE slack_channel_syncs SET rechecked_at=now(),lease_until=now()+interval '5 minutes' \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token=$3 \
         RETURNING json_build_object('ok',true)::text",
        &[team, slack_channel, lease_token],
    )
    .await?;
    if done.is_empty() {
        return Err(SyncError::LeaseLost);
    }
    tx.commit().await.map_err(AppError::from)?;
    Ok(())
}

/// 取り直した投稿のうち、すでに Zoo にあるものに、編集・リアクションの追加・tombstone の削除を反映する
async fn apply_chunk(
    ctx: &mut Context,
    team: &str,
    slack_channel: &str,
    zoo_channel: &str,
    lease_token: &str,
    chunk: &[SlackMessage],
    db: &mut toasty::Db,
) -> Result<(), SyncError> {
    let mut tx = db.transaction().await.map_err(AppError::from)?;
    suppress_app_events(&mut tx).await?;
    let leased = query(
        &mut tx,
        "UPDATE slack_channel_syncs SET lease_until=now()+interval '5 minutes' \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token=$3 \
         RETURNING json_build_object('ok',true)::text",
        &[team, slack_channel, lease_token],
    )
    .await?;
    if leased.is_empty() {
        return Err(SyncError::LeaseLost);
    }
    // `slack_message_map` にある投稿は、その Zoo の ID を使う。Zoo で作った投稿 (`zoo`) は、編集・削除を反映しない
    let tss: Vec<&str> = chunk.iter().map(|m| m.ts.as_str()).collect();
    let mut mapped: HashMap<String, (String, bool)> = HashMap::new();
    for row in query(
        &mut tx,
        "SELECT json_build_object('ts',ts,'id',message_id,'origin',origin)::text FROM slack_message_map \
         WHERE team_id=$1 AND slack_channel_id=$2 AND ts IN (SELECT jsonb_array_elements_text($3::jsonb))",
        &[team, slack_channel, &serde_json::json!(tss).to_string()],
    )
    .await?
    {
        if let (Some(ts), Some(id), Some(origin)) =
            (row["ts"].as_str(), row["id"].as_str(), row["origin"].as_str())
        {
            mapped.insert(ts.to_string(), (id.to_string(), origin == "zoo"));
        }
    }
    let ids: Vec<String> = chunk
        .iter()
        .map(|m| {
            mapped
                .get(&m.ts)
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| super::message_id(team, slack_channel, &m.ts))
        })
        .collect();
    let mut rows: HashMap<String, Value> = HashMap::new();
    for row in query(
        &mut tx,
        "SELECT json_build_object('id',id,'content',content,'card',COALESCE(card,''),'deleted',deleted_at IS NOT NULL)::text \
         FROM messages WHERE channel_id=$2 AND id IN (SELECT jsonb_array_elements_text($1::jsonb))",
        &[&serde_json::json!(ids).to_string(), zoo_channel],
    )
    .await?
    {
        if let Some(id) = row["id"].as_str() {
            rows.insert(id.to_string(), row.clone());
        }
    }
    for (message, id) in chunk.iter().zip(&ids) {
        // Zoo にない投稿は作らない。消えた投稿は、そのまま
        let Some(stored) = rows.get(id).filter(|r| r["deleted"] != true) else {
            continue;
        };
        let zoo_origin = mapped.get(&message.ts).is_some_and(|(_, zoo)| *zoo);
        if message.subtype.as_deref() == Some("tombstone") {
            if !zoo_origin {
                exec_delete(&mut tx, id, zoo_channel).await?;
            }
            continue;
        }
        if !message.is_importable() {
            continue;
        }
        if !zoo_origin {
            edit(&mut tx, ctx, id, message, stored).await?;
        }
        insert_reactions(&mut tx, ctx, id, message).await?;
    }
    tx.commit().await.map_err(AppError::from)?;
    Ok(())
}
