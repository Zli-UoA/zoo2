//! Slack の Events API (#293) が知らせた「確かめ直すメッセージ」の反映。
//!
//! 差分同期の履歴 (`conversations.history`) は、新しい投稿しか返さない。編集・削除・リアクションの変更や、監視範囲
//! (3 日・20 件) の外のスレッドの返信は、イベントで知るしかないが、**イベントの本文は信用しない**。受信口
//! (`events.rs`) は、メッセージ 1 件の指定 (`slack_message_hints`) だけを残し、ここで Slack API からそのメッセージを
//! 取り直して、Zoo を API の今の状態に合わせる。順序の入れ替わり・重複・再送があっても、結果は同じになる。
//!
//! 規則 (それぞれ `backend/tests/slack_hints_test.rs` のテストがある):
//! - **すでに Zoo にあるメッセージだけを変える。** 例外は `reply` (範囲外のスレッドの返信を取り込む)。Zoo にない
//!   メッセージへの編集・削除・リアクションは、何もしない (日付を区切った ZIP の取り込みの前の投稿を、文脈なしに入れない)。
//! - **本文は `changed` のヒントのときだけ書き換える。** 人の投稿は `edited` があるときだけ、その日時が Zoo の
//!   `updated_at` より新しいときに限る (リンクの展開・メタデータの変更では書き換えない。Zoo で編集された本文を、
//!   古い Slack の状態で戻さない)。bot の投稿は、`edited` がなくても、表示内容が違うときだけ。
//! - **削除**: API が「ない」と答えたとき (成功した呼び出しだけ)、または `tombstone` のとき。API がまだメッセージを
//!   返すなら、ヒントを残し、試行回数を増やして、あとの同期で取り直す (上限 5 回・24 時間)。
//! - **リアクションの削除**: 正規化した絵文字 (肌色の違いは 1 つにまとまる) で比べ、API の状態にその人がいないことを
//!   確かめてから、`(message_id, user_id, emoji)` の 1 行だけを消す。`users` が一部しか返らないとき
//!   (`count` のほうが多い) は `reactions.get` (`full=true`) で確かめ、確かめられなければ消さない。

use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    convert::reaction_emoji,
    export::{SlackMessage, SlackReaction},
    sync::{Slack, SyncError, lookup_profiles, parse_messages},
    worker::{
        Context, insert_message, insert_reactions, mapped, render_message, suppress_app_events,
    },
};
use crate::{apps::query, error::AppError};

/// 1 回の同期で取り直すメッセージの上限
const BATCH: usize = 10;
/// 取り直しにかけてよい時間。超えたら、残りは次の同期に回す (リースを切らさないため)
const BUDGET: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
struct Row {
    ts: String,
    kind: String,
    user: String,
    reaction: String,
    thread_ts: String,
    /// 読み込んだときの `updated_at` (テキスト)。処理のあとに消すとき、その後に届いた同じイベントを消さないために使う
    updated_at: String,
}

fn text(row: &Value, key: &str) -> String {
    row[key].as_str().unwrap_or_default().to_string()
}

pub(super) async fn process(
    slack: &mut Slack,
    ctx: &mut Context,
    team: &str,
    slack_channel: &str,
    zoo_channel: &str,
    lease_token: &str,
    db: &mut toasty::Db,
) -> Result<(), SyncError> {
    let mut tx = db.transaction().await.map_err(AppError::from)?;
    let rows = query(
        &mut tx,
        "SELECT json_build_object('ts',message_ts,'kind',kind,'user',slack_user_id,'reaction',reaction,\
         'thread_ts',thread_ts,'attempts',attempts,'updated_at',updated_at)::text \
         FROM slack_message_hints WHERE team_id=$1 AND slack_channel_id=$2 AND attempts<5 \
         AND created_at>now()-interval '24 hours' ORDER BY updated_at,message_ts LIMIT 500",
        &[team, slack_channel],
    )
    .await?;
    tx.commit().await.map_err(AppError::from)?;
    let rows: Vec<Row> = rows
        .iter()
        .map(|r| Row {
            ts: text(r, "ts"),
            kind: text(r, "kind"),
            user: text(r, "user"),
            reaction: text(r, "reaction"),
            thread_ts: text(r, "thread_ts"),
            updated_at: text(r, "updated_at"),
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    // メッセージ (ts) ごとにまとめる。古いヒントから、BATCH 件まで
    let mut order: Vec<String> = Vec::new();
    for row in &rows {
        if !order.contains(&row.ts) {
            order.push(row.ts.clone());
        }
    }
    let started = Instant::now();
    let mut deferred = order.len() > BATCH;
    for ts in order.iter().take(BATCH) {
        if started.elapsed() >= BUDGET {
            deferred = true;
            break;
        }
        let group: Vec<&Row> = rows.iter().filter(|r| &r.ts == ts).collect();
        match apply(
            slack,
            ctx,
            team,
            slack_channel,
            zoo_channel,
            lease_token,
            ts,
            &group,
            db,
        )
        .await
        {
            Ok(()) => {}
            // 待つか、トークンが使えないか、リースを失ったときは、ここで止める (ヒントは残る)
            Err(
                e @ (SyncError::RateLimited(_) | SyncError::NoAccess(_) | SyncError::LeaseLost),
            ) => {
                return Err(e);
            }
            // ほかの失敗は、このメッセージの試行回数を増やして、次へ進む
            Err(SyncError::Failed(error)) => {
                tracing::warn!(team, channel = slack_channel, ts = %ts, %error, "Failed to apply a Slack hint");
                bump(db, team, slack_channel, &group).await?;
            }
            Err(SyncError::InvalidCursor) => bump(db, team, slack_channel, &group).await?,
        }
    }
    // 残りは、次の同期をすぐに起こして続ける
    if deferred {
        let mut tx = db.transaction().await.map_err(AppError::from)?;
        query(
            &mut tx,
            "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,$2) \
             ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() \
             RETURNING json_build_object('ok',true)::text",
            &[team, slack_channel],
        )
        .await?;
        tx.commit().await.map_err(AppError::from)?;
    }
    Ok(())
}

/// このメッセージのヒントの試行回数を増やす
async fn bump(
    db: &mut toasty::Db,
    team: &str,
    slack_channel: &str,
    group: &[&Row],
) -> Result<(), SyncError> {
    let mut tx = db.transaction().await.map_err(AppError::from)?;
    for row in group {
        query(
            &mut tx,
            "UPDATE slack_message_hints SET attempts=attempts+1 WHERE team_id=$1 AND slack_channel_id=$2 \
             AND message_ts=$3 AND kind=$4 AND slack_user_id=$5 AND reaction=$6 \
             RETURNING json_build_object('ok',true)::text",
            &[team, slack_channel, &row.ts, &row.kind, &row.user, &row.reaction],
        )
        .await?;
    }
    tx.commit().await.map_err(AppError::from)?;
    Ok(())
}

/// `conversations.replies` で、`thread` のスレッドのうち `ts` のメッセージだけを求める。
/// 消されたメッセージ・スレッド (`thread_not_found`) は、見つからないものとして `None`
async fn replies(
    slack: &mut Slack,
    channel: &str,
    thread: &str,
    ts: &str,
) -> Result<Option<Value>, SyncError> {
    let params = [
        ("channel", channel),
        ("ts", thread),
        ("oldest", ts),
        ("latest", ts),
        ("inclusive", "true"),
        ("limit", "200"),
    ];
    match slack.get("conversations.replies", &params).await {
        Ok(value) => Ok(Some(value)),
        Err(SyncError::Failed(e)) if e.ends_with("thread_not_found") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Slack にそのメッセージがあるか。API が成功して、見つからなかったときだけ `None`
async fn fetch_message(
    slack: &mut Slack,
    channel: &str,
    ts: &str,
    thread_ts: &str,
) -> Result<Option<SlackMessage>, SyncError> {
    let exact = |value: &Value| parse_messages(value).into_iter().find(|m| m.ts == ts);
    if !thread_ts.is_empty() && thread_ts != ts {
        return Ok(replies(slack, channel, thread_ts, ts)
            .await?
            .and_then(|v| exact(&v)));
    }
    let history = slack
        .get(
            "conversations.history",
            &[
                ("channel", channel),
                ("oldest", ts),
                ("latest", ts),
                ("inclusive", "true"),
                ("limit", "10"),
            ],
        )
        .await?;
    if let Some(message) = exact(&history) {
        return Ok(Some(message));
    }
    // スレッドの返信は、履歴には出ない。`conversations.replies` は、親でも返信でも、スレッドの中のメッセージの
    // `ts` を受け付けるので、`ts` そのもので引く (リアクションのイベントには、親の `ts` がない)
    Ok(replies(slack, channel, ts, ts)
        .await?
        .and_then(|v| exact(&v)))
}

/// 同じメッセージを指すヒントの全部を、1 回の取り直しで反映する
#[allow(clippy::too_many_arguments)]
async fn apply(
    slack: &mut Slack,
    ctx: &mut Context,
    team: &str,
    slack_channel: &str,
    zoo_channel: &str,
    lease_token: &str,
    ts: &str,
    group: &[&Row],
    db: &mut toasty::Db,
) -> Result<(), SyncError> {
    let thread_ts = group
        .iter()
        .map(|r| r.thread_ts.as_str())
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    let has = |kind: &str| group.iter().any(|r| r.kind == kind);
    let fetched = fetch_message(slack, slack_channel, ts, thread_ts).await?;
    // tombstone (返信のある親の削除) は、ヒントの種類によらず、削除として扱う
    let message = fetched.filter(|m| m.subtype.as_deref() != Some("tombstone"));

    // リアクションの削除を反映してよいか。API の状態で、その人のリアクションがないことを確かめる (API の呼び出しは、
    // トランザクションの外で行う)
    let mut removals: Vec<(String, String)> = Vec::new();
    let mut unverified = false;
    if let Some(message) = &message
        && has("reaction")
    {
        let mut reactions = message.reactions.clone();
        let truncated = |rs: &[SlackReaction]| {
            rs.iter()
                .any(|r| r.count.is_some_and(|c| c > r.users.len() as i64))
        };
        if truncated(&reactions) {
            match slack
                .get(
                    "reactions.get",
                    &[
                        ("channel", slack_channel),
                        ("timestamp", ts),
                        ("full", "true"),
                    ],
                )
                .await
            {
                Ok(value) => {
                    reactions = serde_json::from_value(value["message"]["reactions"].clone())
                        .unwrap_or(reactions);
                }
                Err(e @ (SyncError::RateLimited(_) | SyncError::NoAccess(_))) => return Err(e),
                Err(_) => {}
            }
        }
        let complete = !truncated(&reactions);
        for row in group.iter().filter(|r| r.kind == "reaction") {
            let Some(emoji) = reaction_emoji(&row.reaction) else {
                continue;
            };
            let present = reactions.iter().any(|r| {
                reaction_emoji(&r.name).as_deref() == Some(emoji.as_str())
                    && r.users.contains(&row.user)
            });
            if present {
                continue;
            }
            if complete {
                removals.push((row.user.clone(), emoji));
            } else {
                unverified = true;
            }
        }
    }
    if let Some(message) = &message
        && message.is_importable()
    {
        lookup_profiles(db, slack, ctx, std::slice::from_ref(message)).await?;
    }

    let mut tx = db.transaction().await.map_err(AppError::from)?;
    suppress_app_events(&mut tx).await?;
    // 長くかかっても、リースを持ち続ける。失っていたら (別の台が同期を始めた) 何も書かない
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
    // `slack_message_map` に行があれば、その Zoo のメッセージを使う。Zoo で作った投稿 (`zoo`) は Zoo が正なので、
    // Slack での編集・削除は反映しない。Slack からのリアクションと返信は、その投稿に付ける
    let (id, zoo_origin) = match mapped(&mut tx, team, slack_channel, ts).await? {
        Some((id, origin)) => (id, origin == "zoo"),
        None => (super::message_id(team, slack_channel, ts), false),
    };
    let existing = query(
        &mut tx,
        "SELECT json_build_object('deleted',deleted_at IS NOT NULL,'content',content,'card',COALESCE(card,''))::text \
         FROM messages WHERE id=$1 AND channel_id=$2",
        &[&id, zoo_channel],
    )
    .await?
    .into_iter()
    .next();
    // 残すヒント (API がまだ返している削除、確かめられなかったリアクションの削除)
    let mut keep: Vec<&Row> = Vec::new();
    match (&message, &existing) {
        // Slack にもうない。削除のヒントなら (または tombstone)、Zoo のメッセージを論理削除する
        (None, Some(row)) => {
            if !zoo_origin
                && row["deleted"] != true
                && (has("deleted") || has("changed") || has("reaction"))
            {
                // `changed` / `reaction` のヒントで、メッセージが見つからないのは、取り直した時点で消えていたとき。
                // 同じく削除として扱う (API が成功して「ない」と答えている)
                exec_delete(&mut tx, &id, zoo_channel).await?;
            }
        }
        (None, None) => {}
        // Slack にまだある
        (Some(message), existing) => {
            if has("deleted") && !zoo_origin {
                // 削除のイベントが来たのに、まだ返る。あとで取り直す
                keep.extend(group.iter().filter(|r| r.kind == "deleted"));
            }
            match existing {
                None => {
                    if has("reply") && message.is_importable() {
                        insert_message(&mut tx, ctx, slack_channel, zoo_channel, "", message)
                            .await?;
                    }
                }
                Some(row) if row["deleted"] == true => {}
                Some(row) => {
                    if has("changed") && !zoo_origin && message.is_importable() {
                        edit(&mut tx, ctx, &id, message, row).await?;
                    }
                    if has("reaction") || has("changed") {
                        insert_reactions(&mut tx, ctx, &id, message).await?;
                    }
                    for (user, emoji) in &removals {
                        let found = query(
                            &mut tx,
                            "SELECT json_build_object('id',user_id)::text FROM slack_users WHERE team_id=$1 AND slack_user_id=$2",
                            &[team, user],
                        )
                        .await?;
                        if let Some(zoo_user) = found.first().and_then(|r| r["id"].as_str()) {
                            query(
                                &mut tx,
                                "DELETE FROM message_reactions WHERE message_id=$1 AND user_id=$2 AND emoji=$3 \
                                 RETURNING json_build_object('ok',true)::text",
                                &[&id, zoo_user, emoji],
                            )
                            .await?;
                        }
                    }
                }
            }
        }
    }
    if unverified {
        keep.extend(group.iter().filter(|r| r.kind == "reaction"));
    }
    // 処理したヒントを消す。読み込んだあとに届いた同じイベント (`updated_at` が新しい) は残す
    for row in group {
        let kept = keep
            .iter()
            .any(|k| k.kind == row.kind && k.user == row.user && k.reaction == row.reaction);
        let sql = if kept {
            "UPDATE slack_message_hints SET attempts=attempts+1 WHERE team_id=$1 AND slack_channel_id=$2 \
             AND message_ts=$3 AND kind=$4 AND slack_user_id=$5 AND reaction=$6 AND updated_at<=$7::timestamptz \
             RETURNING json_build_object('ok',true)::text"
        } else {
            "DELETE FROM slack_message_hints WHERE team_id=$1 AND slack_channel_id=$2 \
             AND message_ts=$3 AND kind=$4 AND slack_user_id=$5 AND reaction=$6 AND updated_at<=$7::timestamptz \
             RETURNING json_build_object('ok',true)::text"
        };
        query(
            &mut tx,
            sql,
            &[
                team,
                slack_channel,
                &row.ts,
                &row.kind,
                &row.user,
                &row.reaction,
                &row.updated_at,
            ],
        )
        .await?;
    }
    tx.commit().await.map_err(AppError::from)?;
    Ok(())
}

pub(super) async fn exec_delete(
    tx: &mut toasty::Transaction<'_>,
    id: &str,
    zoo_channel: &str,
) -> Result<(), AppError> {
    // ユーザーの削除 (`routes/messages.rs`) と同じく、`deleted_at` と `updated_at` を入れる
    query(
        tx,
        "UPDATE messages SET deleted_at=(now() AT TIME ZONE 'UTC')::timestamp(3),\
         updated_at=(now() AT TIME ZONE 'UTC')::timestamp(3) \
         WHERE id=$1 AND channel_id=$2 AND deleted_at IS NULL RETURNING json_build_object('ok',true)::text",
        &[id, zoo_channel],
    )
    .await?;
    Ok(())
}

/// 編集の反映。本文とカードを、取り込みと同じ変換で作り直し、違うときだけ更新する
pub(super) async fn edit(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    id: &str,
    message: &SlackMessage,
    stored: &Value,
) -> Result<(), AppError> {
    let edited = message
        .edited
        .as_ref()
        .map(|e| e.ts.as_str())
        .filter(|ts| super::is_valid_ts(ts));
    // 人の投稿は、編集の日時があるものだけ。bot の投稿 (`chat.update` には `edited` が付かない) は、表示内容が違うとき
    if edited.is_none() && message.bot_id.is_none() {
        return Ok(());
    }
    let rendered = render_message(tx, ctx, message).await?;
    let same = stored["content"].as_str() == Some(rendered.content.as_str())
        && stored["card"].as_str().unwrap_or_default() == rendered.card;
    if same {
        return Ok(());
    }
    let updated = query(
        tx,
        "UPDATE messages SET content=$2,card=NULLIF($3,''),\
         updated_at=CASE WHEN $4='' THEN (now() AT TIME ZONE 'UTC')::timestamp(3) \
           ELSE (to_timestamp($4::numeric) AT TIME ZONE 'UTC')::timestamp(3) END,\
         revision=revision+1 \
         WHERE id=$1 AND deleted_at IS NULL \
         AND ($4='' OR updated_at IS NULL OR updated_at<(to_timestamp($4::numeric) AT TIME ZONE 'UTC')::timestamp(3)) \
         RETURNING json_build_object('ok',true)::text",
        &[id, &rendered.content, &rendered.card, edited.unwrap_or_default()],
    )
    .await?;
    if updated.is_empty() {
        return Ok(());
    }
    // 補ったファイル名の行を、新しい本文に合わせる (取得できたら、補った名前だけを外すため)
    for file in message.files.iter().filter(|f| f.is_importable()) {
        let in_content = rendered.filled && !ctx.files.will_fetch(file);
        query(
            tx,
            "UPDATE slack_files SET in_content=$3::boolean WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$4 \
             RETURNING json_build_object('ok',true)::text",
            &[
                &ctx.team,
                &file.id,
                if in_content { "true" } else { "false" },
                id,
            ],
        )
        .await?;
    }
    Ok(())
}
