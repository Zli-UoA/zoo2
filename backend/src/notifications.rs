use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{Channel, ChannelUser, Message, Notification, User, now_utc_millis},
};

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
    for event in Parser::new_ext(content, options) {
        match event {
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
    let old_special = old.has_channel || old.has_here;
    let members = ChannelUser::filter(ChannelUser::fields().channel_id().eq(&note.channel_id))
        .exec(tx)
        .await?;
    let member_ids: HashSet<String> = members.into_iter().map(|m| m.user_id).collect();
    let mut old_targets = HashSet::new();
    if old_special {
        old_targets.extend(member_ids.iter().cloned());
    }
    for candidate in old.user_candidates {
        let user = User::get_by_user_name(tx, candidate.raw.as_str())
            .await
            .ok();
        let user = match user {
            Some(user) => Some(user),
            None => {
                if let Some(name) = candidate.trimmed.as_deref() {
                    User::get_by_user_name(tx, name).await.ok()
                } else {
                    None
                }
            }
        };
        if let Some(user) = user {
            if member_ids.contains(&user.id) {
                old_targets.insert(user.id);
            }
        }
    }
    let mut targets = HashSet::new();
    if new.has_channel || new.has_here {
        targets.extend(member_ids.iter().cloned());
    }
    for candidate in new.user_candidates {
        let user = User::get_by_user_name(tx, candidate.raw.as_str())
            .await
            .ok();
        let user = match user {
            Some(user) => Some(user),
            None => {
                if let Some(name) = candidate.trimmed.as_deref() {
                    User::get_by_user_name(tx, name).await.ok()
                } else {
                    None
                }
            }
        };
        if let Some(user) = user
            && member_ids.contains(&user.id)
            && !old_targets.contains(&user.id)
        {
            targets.insert(user.id);
        }
    }
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
    if message.user_id == actor_id {
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
}
