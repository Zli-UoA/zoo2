//! Slack の mrkdwn を zoo の本文 (MFM) に変換する。

use crate::validation::MAX_MESSAGE_CONTENT_LENGTH;

/// `<...>` 内の参照を解決するための対応表
pub trait Resolver {
    /// Slack ユーザー ID → zoo の user_name
    fn user_name(&self, slack_user_id: &str) -> Option<String>;
    /// Slack チャンネル ID → zoo のチャンネル名
    fn channel_name(&self, slack_channel_id: &str) -> Option<String>;
}

/// mrkdwn を MFM に変換する。
///
/// - `<@U123>` / `<@U123|name>` → `@user_name`
/// - `<#C123|name>` → `#name`
/// - `<!here>` / `<!channel>` / `<!everyone>` → `@here` など
/// - `<!subteam^S1|@team>` → `@team`
/// - `<https://a|label>` → `[label](https://a)`、`<https://a>` → `https://a`
/// - `&amp;` `&lt;` `&gt;` を元に戻す
pub fn mrkdwn_to_mfm(text: &str, resolver: &impl Resolver) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&unescape(&rest[..start]));
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            out.push_str(&unescape(&rest[start..]));
            rest = "";
            break;
        };
        out.push_str(&convert_token(&after[..end], resolver));
        rest = &after[end + 1..];
    }
    out.push_str(&unescape(rest));
    truncate_content(out)
}

/// 本文でメンションされている Slack ユーザー ID (`<@U123>` / `<@W123|name>`) を出現順・重複なしで返す
pub fn mentioned_users(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (index, _) in text.match_indices("<@") {
        let token = &text[index + 2..];
        let end = token.find(['>', '|']).unwrap_or(token.len());
        let id = &token[..end];
        let valid = id.len() > 1
            && id.starts_with(['U', 'W'])
            && id
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if valid && !found.iter().any(|f| f == id) {
            found.push(id.to_string());
        }
    }
    found
}

fn convert_token(token: &str, resolver: &impl Resolver) -> String {
    let (target, label) = match token.split_once('|') {
        Some((t, l)) => (t, Some(unescape(l))),
        None => (token, None),
    };
    if let Some(user) = target.strip_prefix('@') {
        return match resolver.user_name(user) {
            Some(name) => format!("@{name}"),
            None => format!("@{}", label.unwrap_or_else(|| user.to_string())),
        };
    }
    if let Some(channel) = target.strip_prefix('#') {
        let name = resolver
            .channel_name(channel)
            .or(label)
            .unwrap_or_else(|| channel.to_string());
        return format!("#{name}");
    }
    if let Some(special) = target.strip_prefix('!') {
        return match special {
            "here" | "channel" | "everyone" => format!("@{special}"),
            _ if special.starts_with("subteam^") => label.unwrap_or_else(|| "@team".to_string()),
            _ => label.unwrap_or_default(),
        };
    }
    let url = unescape(target);
    match label {
        Some(label) if !label.is_empty() && label != url => {
            if is_web_url(&url) {
                format!("[{}]({url})", label.replace(['[', ']'], ""))
            } else {
                label
            }
        }
        _ => url
            .strip_prefix("mailto:")
            .map(str::to_string)
            .unwrap_or(url),
    }
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// zoo の本文上限に合わせて切り詰める
pub fn truncate_content(s: String) -> String {
    if s.chars().count() <= MAX_MESSAGE_CONTENT_LENGTH {
        return s;
    }
    s.chars().take(MAX_MESSAGE_CONTENT_LENGTH).collect()
}

/// Slack のリアクション名 (`+1::skin-tone-2` など) を zoo の絵文字表記に正規化する
pub fn reaction_emoji(name: &str) -> Option<String> {
    let base = name.split("::").next().unwrap_or(name).to_ascii_lowercase();
    crate::validation::normalize_and_validate_emoji(&format!(":{base}:")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Map;
    impl Resolver for Map {
        fn user_name(&self, id: &str) -> Option<String> {
            (id == "U1").then(|| "alice".to_string())
        }
        fn channel_name(&self, id: &str) -> Option<String> {
            (id == "C1").then(|| "random".to_string())
        }
    }

    #[test]
    fn collects_mentioned_users_once() {
        assert_eq!(
            mentioned_users("<@U1> <@W2|bob> <@U1> <@lower> <#C1> <@U3"),
            vec!["U1", "W2", "U3"]
        );
    }

    #[test]
    fn converts_mentions_channels_and_links() {
        let text = "hi <@U1> and <@U9|bob> in <#C1|general> &amp; <#C9|other> <!here> \
                    <https://example.com|Example> <https://x.test> <mailto:a@b.c|a@b.c> \
                    <!subteam^S1|@devs> 1 &lt; 2";
        assert_eq!(
            mrkdwn_to_mfm(text, &Map),
            "hi @alice and @bob in #random & #other @here \
             [Example](https://example.com) https://x.test a@b.c \
             @devs 1 < 2"
        );
    }

    #[test]
    fn keeps_unclosed_brackets_and_non_web_links() {
        assert_eq!(mrkdwn_to_mfm("a < b", &Map), "a < b");
        assert_eq!(mrkdwn_to_mfm("<slack://open|Open>", &Map), "Open");
    }

    #[test]
    fn truncates_long_content() {
        let long = "a".repeat(MAX_MESSAGE_CONTENT_LENGTH + 10);
        assert_eq!(
            mrkdwn_to_mfm(&long, &Map).chars().count(),
            MAX_MESSAGE_CONTENT_LENGTH
        );
    }

    #[test]
    fn normalizes_reaction_names() {
        assert_eq!(reaction_emoji("+1::skin-tone-2").as_deref(), Some("👍"));
        assert_eq!(
            reaction_emoji("partyparrot").as_deref(),
            Some(":partyparrot:")
        );
        assert_eq!(reaction_emoji("bad name!"), None);
    }
}
