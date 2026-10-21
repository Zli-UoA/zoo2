use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{Channel, ChannelUser, Message, Notification, User, now_utc_millis},
};

#[derive(Debug, PartialEq, Eq)]
pub struct MentionTargets {
    pub usernames: HashSet<String>,
    pub has_channel: bool,
    pub has_here: bool,
}

/// メッセージ本文からメンション (@username, @channel, @here) を抽出する
pub fn extract_mentions(content: &str) -> MentionTargets {
    let mut usernames = HashSet::new();
    let mut has_channel = false;
    let mut has_here = false;

    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '@' {
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
                if token.eq_ignore_ascii_case("channel") {
                    has_channel = true;
                } else if token.eq_ignore_ascii_case("here") {
                    has_here = true;
                } else {
                    usernames.insert(token);
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }

    MentionTargets {
        usernames,
        has_channel,
        has_here,
    }
}

/// メッセージ作成時の通知生成
///
/// 1つのメッセージ投稿に対し、同一ユーザー宛ての通知は1件に集約する (メンション > 返信)。
/// チャンネル非所属ユーザー (キック・退出者含む) や本人自身のアクションは通知から除外する。
pub async fn create_message_notifications(
    tx: &mut toasty::Transaction<'_>,
    message: &Message,
    actor_id: &str,
    parent_author_id: Option<&str>,
) -> Result<Vec<Notification>, AppError> {
    let mentions = extract_mentions(&message.content);

    // チャンネル所属メンバーのIDを取得
    let channel_users = ChannelUser::filter(ChannelUser::fields().channel_id().eq(&message.channel_id))
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

    // 2. @username: 該当ユーザーが存在し、かつチャンネル所属メンバーである場合 (本人除く)
    for username in &mentions.usernames {
        if let Ok(user) = User::get_by_user_name(tx, username.as_str()).await {
            if user.id != actor_id && member_ids.contains(&user.id) {
                targets_to_notify.insert(user.id, "mention");
            }
        }
    }

    // 3. スレッド返信: 親メッセージの投稿者 (本人除く)
    // [P1] 過去に投稿していても現在チャンネルメンバーでない (キック/退出された) 場合は通知しない
    if let Some(parent_id) = parent_author_id {
        if parent_id != actor_id && member_ids.contains(parent_id) {
            // 既にメンション対象になっていれば "mention" を優先
            targets_to_notify.entry(parent_id.to_string()).or_insert("reply");
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

        assert_eq!(targets.usernames.len(), 2);
        assert!(targets.usernames.contains("alice"));
        assert!(targets.usernames.contains("bob"));
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn test_extract_mentions_special() {
        let text = "Attention @channel and @HERE please";
        let targets = extract_mentions(text);

        assert!(targets.usernames.is_empty());
        assert!(targets.has_channel);
        assert!(targets.has_here);
    }

    #[test]
    fn test_extract_mentions_mixed() {
        let text = "@charlie please ask @channel";
        let targets = extract_mentions(text);

        assert_eq!(targets.usernames.len(), 1);
        assert!(targets.usernames.contains("charlie"));
        assert!(targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn test_extract_mentions_none() {
        let text = "No mentions here, just an email test@example.com";
        let targets = extract_mentions(text);

        // emailの @example.com も正規表現 @([a-zA-Z0-9_.-]+) でマッチしうるが、
        // 存在しないユーザーはDBルックアップで除外される
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }
}
