//! 暗号化チャンネルの送信に付ける平文の `mentions`（docs/e2ee.md の「メッセージ」）。
//!
//! サーバーは暗号化した本文を読めないので、平文チャンネルでサーバーが本文から作るのと同じ規則
//! （`backend/src/notifications.rs` の `extract_mentions`）で、送信側が作る。React の
//! `components/chat/messages/mentions.ts` の `encryptedMentions` と同じ。

use std::collections::{BTreeSet, HashMap};

/// メンション（`{user_ids, channel, here}`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mentions {
    /// メンションしたユーザー ID（昇順、重複なし）。
    pub user_ids: Vec<String>,
    /// `@channel`。
    pub channel: bool,
    /// `@here`。
    pub here: bool,
}

/// ユーザー名の末尾から外す句読点（`backend/src/notifications.rs` の `PUNCTUATION` と同じ）。
const PUNCTUATION: &[char] = &['.', ',', '!', '?', ':', ';', '。', '、', '！', '？'];

/// `@` の直前に来てよい文字（行頭のほか）。`backend/src/notifications.rs` の `is_mention_boundary` と同じ。
fn is_boundary(prev: char) -> bool {
    prev.is_whitespace()
        || matches!(
            prev,
            '(' | '[' | '{' | '<' | '（' | '「' | '『' | '"' | '\'' | '。' | '、'
        )
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
}

/// 本文の `@name`、`@channel`、`@here` から `mentions` を作る。
///
/// `users` は `(ユーザー ID, ユーザー名)`。ユーザー名は、語のまま、なければ末尾の句読点を外して引く。
/// 引けない名前は入れない。`@channel` と `@here` は大文字と小文字を区別しない。
pub fn encrypted_mentions<'a>(
    content: &str,
    users: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Mentions {
    let by_name: HashMap<&str, &str> = users.into_iter().map(|(id, name)| (name, id)).collect();
    let chars: Vec<char> = content.chars().collect();
    let mut user_ids = BTreeSet::new();
    let mut mentions = Mentions::default();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || is_boundary(chars[i - 1])) {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && is_name_char(chars[end]) {
                end += 1;
            }
            if end > start {
                let raw: String = chars[start..end].iter().collect();
                let trimmed = raw.trim_end_matches(PUNCTUATION);
                if trimmed.eq_ignore_ascii_case("channel") {
                    mentions.channel = true;
                } else if trimmed.eq_ignore_ascii_case("here") {
                    mentions.here = true;
                } else if let Some(id) = by_name.get(raw.as_str()).or_else(|| by_name.get(trimmed))
                {
                    user_ids.insert((*id).to_owned());
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    mentions.user_ids = user_ids.into_iter().collect();
    mentions
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERS: [(&str, &str); 3] = [("u1", "alice"), ("u2", "bob.k"), ("u3", "carol")];

    #[test]
    fn finds_users_channel_and_here_like_the_server() {
        let mentions = encrypted_mentions("@alice と @bob.k. @Channel (@here) @nobody", USERS);
        assert_eq!(mentions.user_ids, vec!["u1", "u2"]);
        assert!(mentions.channel);
        assert!(mentions.here);
    }

    #[test]
    fn ignores_mail_addresses_and_unknown_names() {
        let mentions = encrypted_mentions("mail carol@example.com, @dave", USERS);
        assert_eq!(mentions, Mentions::default());
    }

    #[test]
    fn deduplicates_and_sorts() {
        let mentions = encrypted_mentions("@carol @alice、@carol！", USERS);
        assert_eq!(mentions.user_ids, vec!["u1", "u3"]);
    }
}
