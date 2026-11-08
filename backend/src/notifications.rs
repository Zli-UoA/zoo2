use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{Channel, ChannelUser, Message, Notification, User, now_utc_millis},
};

pub(crate) async fn create_federated_notification(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    actor: &str,
    activity: &str,
    kind: &str,
    post: &str,
) -> Result<(), AppError> {
    toasty::sql::statement("INSERT INTO notification_events(id,user_id,actor_id,activity_id,notification_type,post_id,source) VALUES($1,$2,$3,$4,$5,NULLIF($6,''),'activitypub') ON CONFLICT DO NOTHING")
        .bind(uuid::Uuid::new_v4().to_string()).bind(user).bind(actor).bind(activity).bind(kind).bind(post).exec(tx).await?;
    Ok(())
}
pub struct InboxReadResult {
    pub updated_count: usize,
    /// Only return an Electric acknowledgement when the channel Shape can observe it.
    pub channel_changed: bool,
}

/// Shared inbox mutation for channel and federated notifications. A missing selection
/// means all matching rows; an empty selection is deliberately a no-op.
pub async fn mark_read(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    ids: Option<&[String]>,
    source: Option<&str>,
    channel: Option<&str>,
) -> Result<InboxReadResult, AppError> {
    if ids.is_some_and(|ids| ids.len() > 500) {
        return Err(AppError::BadRequest(
            "Too many notifications selected".into(),
        ));
    }
    let ids_json = serde_json::json!(ids).to_string();
    let rows = toasty::sql::query(
        "UPDATE notification_events SET is_read=true
         WHERE user_id=$1 AND NOT is_read
           AND ($2::jsonb='null'::jsonb OR id IN (
             SELECT jsonb_array_elements_text(CASE WHEN $2::jsonb='null'::jsonb THEN '[]'::jsonb ELSE $2::jsonb END)))
           AND ($3='' OR source=$3) AND ($4='' OR channel_id=$4)
         RETURNING source",
    )
    .bind(user)
    .bind(ids_json)
    .bind(source.unwrap_or_default())
    .bind(channel.unwrap_or_default())
    .exec(tx)
    .await?;
    let channel_changed = rows.iter().any(|row| {
        matches!(row, toasty::stmt::Value::Record(record)
            if matches!(record.fields.first(), Some(toasty::stmt::Value::String(source)) if source == "channel"))
    });
    Ok(InboxReadResult {
        updated_count: rows.len(),
        channel_changed,
    })
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MentionUserCandidate {
    pub raw: String,
    pub trimmed: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MentionTargets {
    pub user_candidates: Vec<MentionUserCandidate>,
    pub has_channel: bool,
    pub has_here: bool,
}

const PUNCTUATION: &[char] = &['.', ',', '!', '?', ':', ';', '。', '、', '！', '？'];

pub fn is_mention_boundary(prev: char) -> bool {
    prev.is_whitespace()
        || matches!(
            prev,
            '(' | '[' | '{' | '<' | '（' | '「' | '『' | '"' | '\'' | '。' | '、'
        )
}

/// メッセージ本文からメンション (@username, @channel, @here) を抽出する
///
/// [P3] @ の直前が行頭・空白・開き括弧などの区切り文字であることを要求する (メールアドレスやURLの除外)。
/// また、Thanks @bob. のように文末に句読点がある場合でもユーザー名候補として照合できるよう、
/// 完全一致候補 (raw) と末尾句読点トリム候補 (trimmed) の双方を保持する。
pub fn extract_mentions(content: &str) -> MentionTargets {
    let mut user_candidates = Vec::new();
    let mut has_channel = false;
    let mut has_here = false;

    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '@' {
            let is_boundary = i == 0 || is_mention_boundary(chars[i - 1]);
            if is_boundary {
                let start = i + 1;
                let mut end = start;
                while end < len {
                    let c = chars[end];
                    if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                        end += 1;
                    } else {
                        break;
                    }
                }

                if end > start {
                    let token: String = chars[start..end].iter().collect();
                    let trimmed_token = token.trim_end_matches(PUNCTUATION);

                    if trimmed_token.eq_ignore_ascii_case("channel") {
                        has_channel = true;
                    } else if trimmed_token.eq_ignore_ascii_case("here") {
                        has_here = true;
                    } else {
                        let trimmed = if trimmed_token != token && !trimmed_token.is_empty() {
                            Some(trimmed_token.to_string())
                        } else {
                            None
                        };
                        user_candidates.push(MentionUserCandidate {
                            raw: token,
                            trimmed,
                        });
                    }
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }

    MentionTargets {
        user_candidates,
        has_channel,
        has_here,
    }
}

/// Markdown本文の表示テキストだけからメンションを抽出する。コードとリンク先URLは除外する。
pub fn extract_markdown_mentions(content: &str) -> MentionTargets {
    let mut visible = String::with_capacity(content.len());
    // pulldown-cmark's ENABLE_GFM currently covers blockquote alerts only.
    // Keep these aligned with remark-gfm's table, strikethrough, and task list parsing.
    let options = Options::ENABLE_GFM
        | Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let mut in_code_block = false;
    let mut image_depth = 0usize;
    for event in Parser::new_ext(content, options) {
        match event {
            Event::Start(Tag::Image { .. }) => {
                image_depth += 1;
                visible.push(' ');
            }
            Event::End(TagEnd::Image) => {
                image_depth = image_depth.saturating_sub(1);
                visible.push(' ');
            }
            _ if image_depth > 0 => {}
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                visible.push(' ');
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                visible.push(' ');
            }
            Event::Text(text) if !in_code_block => visible.push_str(&text),
            Event::SoftBreak | Event::HardBreak => visible.push('\n'),
            // Code spans, link destinations, and raw HTML markup are not visible mention text.
            Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) => visible.push(' '),
            Event::Start(_) | Event::End(_) => visible.push(' '),
            _ => {}
        }
    }
    extract_mentions(&visible)
}

/// 保存済み本文から新規に加わったメンションだけに通知する。
pub async fn create_note_notifications(
    tx: &mut toasty::Transaction<'_>,
    note: &crate::models::ChannelNote,
    previous_content: &str,
    new_content: &str,
    actor_id: &str,
) -> Result<Vec<Notification>, AppError> {
    let old = extract_markdown_mentions(previous_content);
    let new = extract_markdown_mentions(new_content);
    if new.user_candidates.is_empty() && !new.has_channel && !new.has_here {
        return Ok(Vec::new());
    }
    // Resolve each distinct spelling once, preserving exact-name precedence over punctuation trimming.
    let names: HashSet<&str> = old
        .user_candidates
        .iter()
        .chain(&new.user_candidates)
        .flat_map(|candidate| {
            std::iter::once(candidate.raw.as_str()).chain(candidate.trimmed.as_deref())
        })
        .collect();
    let mut users_by_name = HashMap::new();
    if !names.is_empty() {
        let encoded = serde_json::json!(names).to_string();
        for user in crate::apps::query(tx,
            "SELECT json_build_object('name',user_name,'id',id)::text FROM users WHERE user_name IN (SELECT jsonb_array_elements_text($1::jsonb))",
            &[&encoded]).await? {
            if let (Some(name), Some(id)) = (user["name"].as_str(), user["id"].as_str()) {
                users_by_name.insert(name.to_owned(), id.to_owned());
            }
        }
    }
    // Bots consume app events, not human inbox notifications, just as for messages.
    let members = crate::apps::query(tx,
        "SELECT json_build_object('id',u.id)::text FROM channel_user cu JOIN users u ON u.id=cu.user_id WHERE cu.channel_id=$1 AND NOT u.is_bot",
        &[&note.channel_id]).await?;
    let member_ids: HashSet<String> = members
        .into_iter()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    let resolve = |mentions: MentionTargets| {
        let mut targets = HashSet::new();
        if mentions.has_channel || mentions.has_here {
            targets.extend(member_ids.iter().cloned());
        }
        for candidate in mentions.user_candidates {
            if let Some(id) = users_by_name.get(&candidate.raw).or_else(|| {
                candidate
                    .trimmed
                    .as_ref()
                    .and_then(|name| users_by_name.get(name))
            }) && member_ids.contains(id)
            {
                targets.insert(id.clone());
            }
        }
        targets
    };
    let old_targets = resolve(old);
    let mut targets = resolve(new);
    targets.retain(|user_id| !old_targets.contains(user_id));
    let mut created = Vec::with_capacity(targets.len());
    for user_id in targets.into_iter().filter(|id| id != actor_id) {
        let notification = Notification::create()
            .id(Uuid::new_v4().to_string())
            .user_id(user_id)
            .actor_id(actor_id.to_string())
            .channel_id(note.channel_id.clone())
            .message_id(None)
            .note_id(Some(note.id.clone()))
            .notification_type("mention".to_string())
            .is_read(false)
            .created_at(now_utc_millis())
            .exec(tx)
            .await?;
        created.push(notification);
    }
    Ok(created)
}

/// メッセージ作成時の通知生成
///
/// 1つのメッセージ投稿に対し、同一ユーザー宛ての通知は1件に集約する (メンション > 返信 > DM)。
/// [P2] 公開チャンネルでは非メンバー投稿者への返信通知も配信する。
/// プライベートチャンネルではチャンネル非所属ユーザー (キック・退出者含む) への通知を除外する。
pub async fn create_message_notifications(
    tx: &mut toasty::Transaction<'_>,
    message: &Message,
    actor_id: &str,
    parent_author_id: Option<&str>,
) -> Result<Vec<Notification>, AppError> {
    let mentions = extract_mentions(&message.content);

    // チャンネル情報を取得 (公開かプライベートかで通知先ポリシーを判定)
    let channel = Channel::get_by_id(tx, message.channel_id.as_str()).await?;

    // チャンネル所属メンバーのIDを取得
    let channel_users =
        ChannelUser::filter(ChannelUser::fields().channel_id().eq(&message.channel_id))
            .exec(tx)
            .await?;
    let member_ids: HashSet<String> = channel_users.into_iter().map(|cu| cu.user_id).collect();

    // 通知対象ユーザーと通知タイプ (user_id -> notification_type)
    let mut targets_to_notify: HashMap<String, &'static str> = HashMap::new();

    // 1. @channel / @here: チャンネル所属メンバー全員 (本人除く)
    if mentions.has_channel || mentions.has_here {
        for member_id in &member_ids {
            if member_id != actor_id {
                targets_to_notify.insert(member_id.clone(), "mention");
            }
        }
    }

    // 2. @username: 該当ユーザーが存在する場合 (本人除く)
    // [P3] raw (完全一致) で引いた後、見つからなければ trimmed (末尾句読点除去) で引く
    for candidate in mentions.user_candidates {
        let user_opt = if let Ok(u) = User::get_by_user_name(tx, candidate.raw.as_str()).await {
            Some(u)
        } else if let Some(ref trimmed) = candidate.trimmed {
            User::get_by_user_name(tx, trimmed.as_str()).await.ok()
        } else {
            None
        };

        if let Some(user) = user_opt
            && user.id != actor_id
            && member_ids.contains(&user.id)
        {
            targets_to_notify.insert(user.id, "mention");
        }
    }

    // 3. スレッド返信: 親メッセージの投稿者 (本人除く)
    // [P2] 公開チャンネルなら非メンバー投稿者であっても返信通知を作成する
    // プライベートチャンネルの場合は channel_user 所属を確認し、キック/退出者には通知しない
    if let Some(parent_id) = parent_author_id
        && parent_id != actor_id
    {
        let can_notify = if channel.is_private {
            member_ids.contains(parent_id)
        } else {
            channel.deleted_at.is_none()
        };

        if can_notify {
            // 既にメンション対象になっていれば "mention" を優先
            targets_to_notify
                .entry(parent_id.to_string())
                .or_insert("reply");
        }
    }

    // 通常の DM も現在の参加者全員へ通知する。メンション・返信は優先して重複させない。
    if channel.is_dm && channel.deleted_at.is_none() {
        for member_id in &member_ids {
            if member_id != actor_id {
                targets_to_notify.entry(member_id.clone()).or_insert("dm");
            }
        }
    }

    // Apps receive events rather than human inbox notifications.
    if !targets_to_notify.is_empty() {
        let ids = serde_json::json!(targets_to_notify.keys().collect::<Vec<_>>()).to_string();
        let bots = crate::apps::query(tx, "SELECT json_build_object('id',id)::text FROM users WHERE is_bot AND id IN (SELECT jsonb_array_elements_text($1::jsonb))", &[&ids]).await?;
        for bot in bots {
            if let Some(id) = bot["id"].as_str() {
                targets_to_notify.remove(id);
            }
        }
    }

    let mut created_notifications = Vec::new();
    let now = now_utc_millis();

    for (target_user_id, notif_type) in targets_to_notify {
        let notification = Notification::create()
            .id(Uuid::new_v4().to_string())
            .user_id(target_user_id)
            .actor_id(actor_id.to_string())
            .channel_id(message.channel_id.clone())
            .message_id(message.id.clone())
            .notification_type(notif_type.to_string())
            .is_read(false)
            .created_at(now)
            .exec(tx)
            .await?;
        created_notifications.push(notification);
    }

    Ok(created_notifications)
}

/// リアクション追加時の通知生成
///
/// 本人のメッセージに対するリアクションは通知しない。
/// [P1] メッセージ投稿者が現在チャンネルを閲覧可能か確認し、キック・退出済みの場合は通知しない。
pub async fn create_reaction_notification(
    tx: &mut toasty::Transaction<'_>,
    message: &Message,
    actor_id: &str,
) -> Result<Option<Notification>, AppError> {
    if message.user_id == actor_id || User::get_by_id(tx, message.user_id.as_str()).await?.is_bot {
        return Ok(None);
    }

    let channel = Channel::get_by_id(tx, message.channel_id.as_str()).await?;
    if channel.is_private {
        let is_member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(&message.channel_id)
                .and(ChannelUser::fields().user_id().eq(&message.user_id)),
        )
        .first()
        .exec(tx)
        .await?
        .is_some();

        if !is_member {
            return Ok(None);
        }
    }

    let notification = Notification::create()
        .id(Uuid::new_v4().to_string())
        .user_id(message.user_id.clone())
        .actor_id(actor_id.to_string())
        .channel_id(message.channel_id.clone())
        .message_id(message.id.clone())
        .notification_type("reaction".to_string())
        .is_read(false)
        .created_at(now_utc_millis())
        .exec(tx)
        .await?;

    Ok(Some(notification))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_mentions_basic() {
        let text = "Hello @alice and @bob, check this out!";
        let targets = extract_mentions(text);

        assert_eq!(targets.user_candidates.len(), 2);
        assert_eq!(targets.user_candidates[0].raw, "alice");
        assert_eq!(targets.user_candidates[0].trimmed, None);
        assert_eq!(targets.user_candidates[1].raw, "bob");
        assert_eq!(targets.user_candidates[1].trimmed, None);
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn test_extract_mentions_punctuation_and_sentence_end() {
        let text = "Thanks @bob. Have you seen @charlie! Also (@dave)";
        let targets = extract_mentions(text);

        assert_eq!(targets.user_candidates.len(), 3);
        assert_eq!(targets.user_candidates[0].raw, "bob.");
        assert_eq!(targets.user_candidates[0].trimmed, Some("bob".to_string()));

        assert_eq!(targets.user_candidates[1].raw, "charlie");
        assert_eq!(targets.user_candidates[1].trimmed, None);

        assert_eq!(targets.user_candidates[2].raw, "dave");
        assert_eq!(targets.user_candidates[2].trimmed, None);
    }

    #[test]
    fn test_extract_mentions_special_with_punctuation() {
        let text = "Attention @channel! and @HERE. please";
        let targets = extract_mentions(text);

        assert!(targets.user_candidates.is_empty());
        assert!(targets.has_channel);
        assert!(targets.has_here);
    }

    #[test]
    fn test_extract_mentions_email_and_url_ignored() {
        let text = "Contact test@example.com or visit https://example.com/@here or @channel_inside";
        let targets = extract_mentions(text);

        // test@example.com -> 前が 't' なので境界でないため無視
        // https://example.com/@here -> 前が '/' なので境界でないため無視
        // @channel_inside -> 前が ' ' なので有効なメンショントークン
        assert!(!targets.has_here);
        assert_eq!(targets.user_candidates.len(), 1);
        assert_eq!(targets.user_candidates[0].raw, "channel_inside");
    }

    #[test]
    fn test_extract_mentions_none() {
        let text = "No mentions here, just normal text";
        let targets = extract_mentions(text);

        assert!(targets.user_candidates.is_empty());
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn test_extract_mentions_japanese_punctuation() {
        let text = "こんにちは。@bob、確認してください、@alice！";
        let targets = extract_mentions(text);

        assert_eq!(targets.user_candidates.len(), 2);
        assert_eq!(targets.user_candidates[0].raw, "bob");
        assert_eq!(targets.user_candidates[1].raw, "alice");
    }

    #[test]
    fn markdown_mentions_match_visible_escaped_text_and_ignore_code_and_link_destinations() {
        let targets = extract_markdown_mentions(
            "@alice `@code`\n\n```txt\n@block\n```\n\n[site](https://example.test/@url) \\@escaped @bob",
        );
        assert_eq!(
            targets
                .user_candidates
                .iter()
                .map(|c| c.raw.as_str())
                .collect::<Vec<_>>(),
            ["alice", "escaped", "bob"]
        );
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn markdown_mentions_respect_special_targets_and_tilde_code_fences() {
        let targets =
            extract_markdown_mentions("@channel and @here\n\n~~~text\n@ignored\n~~~\n@alice");
        assert!(targets.has_channel);
        assert!(targets.has_here);
        assert_eq!(
            targets
                .user_candidates
                .iter()
                .map(|candidate| candidate.raw.as_str())
                .collect::<Vec<_>>(),
            ["alice"]
        );
    }

    #[test]
    fn markdown_mentions_follow_markdown_structure_for_reference_links() {
        let targets = extract_markdown_mentions(
            "[visible @alice][ref] and https://example.test/@hidden\n\n[ref]: @also_hidden",
        );
        assert_eq!(
            targets
                .user_candidates
                .iter()
                .map(|candidate| candidate.raw.as_str())
                .collect::<Vec<_>>(),
            ["alice"]
        );
    }

    #[test]
    fn markdown_mentions_match_gfm_visible_text() {
        let targets = extract_markdown_mentions(
            "@plain\n\n    @indented\n\n```txt\n@fenced3\n```\n\n````txt\n@fenced4\n````\n\n`@inline` and [label @linked][ref]\n\n[ref]: https://example.test/@destination\n\n\\@escaped &commat;entity <b>@html</b>",
        );
        assert_eq!(
            targets
                .user_candidates
                .iter()
                .map(|candidate| candidate.raw.as_str())
                .collect::<Vec<_>>(),
            ["plain", "linked", "escaped", "entity", "html"]
        );
    }

    #[test]
    fn markdown_mentions_match_visible_gfm_table_and_strikethrough_text() {
        let targets = extract_markdown_mentions(
            "| user | status |\n| --- | --- |\n| @alice | ~~@bob~~ |\n\n@plain `@code` [label @linked](https://example.test/@destination)",
        );
        assert_eq!(
            targets
                .user_candidates
                .iter()
                .map(|candidate| candidate.raw.as_str())
                .collect::<Vec<_>>(),
            ["alice", "bob", "plain", "linked"]
        );
    }

    #[test]
    fn markdown_mentions_exclude_image_alt_text() {
        let targets = extract_markdown_mentions("![@hidden](https://example.test/p.png) @visible");
        assert_eq!(
            targets
                .user_candidates
                .iter()
                .map(|c| c.raw.as_str())
                .collect::<Vec<_>>(),
            ["visible"]
        );
    }
}
