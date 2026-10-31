use std::collections::HashMap;
use std::sync::LazyLock;

use crate::error::AppError;

pub const MAX_CHANNEL_NAME_LENGTH: usize = 80;
pub const MAX_CHANNEL_DESCRIPTION_LENGTH: usize = 250;
pub const MAX_USER_NAME_LENGTH: usize = 80;
pub const MAX_DISPLAY_NAME_LENGTH: usize = 80;
pub const MAX_MESSAGE_CONTENT_LENGTH: usize = 40_000;

const RESERVED_CHANNEL_NAMES: &[&str] = &["all", "channel", "everyone", "here", "general"];
pub(crate) const RESERVED_USER_NAMES: &[&str] =
    &["all", "channel", "everyone", "here", "slackbot", "admin"];

/// チャンネル名のバリデーション:
/// - 1 文字以上 80 文字以下
/// - Unicode の文字・結合文字・数字と半角ハイフン (`-`)・アンダースコア (`_`) のみ許可
/// - 予約語 (`all`, `channel`, `everyone`, `here`, `general`) の禁止
pub fn validate_channel_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Channel name cannot be empty or only whitespace".to_string(),
        ));
    }

    if name.chars().count() > MAX_CHANNEL_NAME_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Channel name cannot exceed {MAX_CHANNEL_NAME_LENGTH} characters"
        )));
    }

    if RESERVED_CHANNEL_NAMES.contains(&name) {
        return Err(AppError::BadRequest(format!(
            "Channel name '{name}' is reserved"
        )));
    }

    static CHANNEL_NAME_PATTERN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\A[\p{L}\p{M}\p{N}_-]+\z").unwrap());
    if !CHANNEL_NAME_PATTERN.is_match(name) {
        return Err(AppError::BadRequest(
            "Channel name can only contain letters, combining marks, numbers, hyphens, and underscores"
                .to_string(),
        ));
    }

    Ok(())
}

/// チャンネル説明のバリデーション (Slack 準拠):
/// - 最大 250 文字 (None の場合は許容)
pub fn validate_channel_description(desc: Option<&str>) -> Result<(), AppError> {
    if let Some(d) = desc
        && d.chars().count() > MAX_CHANNEL_DESCRIPTION_LENGTH
    {
        return Err(AppError::BadRequest(format!(
            "Channel description cannot exceed {MAX_CHANNEL_DESCRIPTION_LENGTH} characters"
        )));
    }
    Ok(())
}

/// ユーザー名のバリデーション (Slack 準拠):
/// - 1 文字以上 80 文字以下
/// - 半角英小文字、数字、ピリオド (`.`)、ハイフン (`-`)、アンダースコア (`_`) のみ許可
/// - 予約語 (`all`, `channel`, `everyone`, `here`, `slackbot`, `admin`) の禁止
pub fn validate_user_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest("Username cannot be empty".to_string()));
    }

    if name.chars().count() > MAX_USER_NAME_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Username cannot exceed {MAX_USER_NAME_LENGTH} characters"
        )));
    }

    if RESERVED_USER_NAMES.contains(&name) {
        return Err(AppError::BadRequest(format!(
            "Username '{name}' is reserved"
        )));
    }

    let is_valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' || c == '_');

    if !is_valid {
        return Err(AppError::BadRequest(
            "Username can only contain lowercase letters, numbers, periods, hyphens, and underscores"
                .to_string(),
        ));
    }

    Ok(())
}

/// 表示名のバリデーション (Slack 準拠):
/// - トリム後 1 文字以上 80 文字以下
pub fn validate_display_name(name: &str) -> Result<(), AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "Display name cannot be empty or only whitespace".to_string(),
        ));
    }

    if trimmed.chars().count() > MAX_DISPLAY_NAME_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Display name cannot exceed {MAX_DISPLAY_NAME_LENGTH} characters"
        )));
    }

    Ok(())
}

/// メッセージ本文のバリデーション (Slack 準拠):
/// - 空文字・空白のみのメッセージは禁止
/// - 最大 40,000 文字
pub fn validate_message_content(content: &str) -> Result<(), AppError> {
    validate_message_content_with_options(content, false)
}

/// メッセージ本文のバリデーション (空本文許容オプション付き):
/// - `allow_empty` が `false` の場合、空文字・空白のみのメッセージは禁止
/// - 最大 40,000 文字は常に検証
pub fn validate_message_content_with_options(
    content: &str,
    allow_empty: bool,
) -> Result<(), AppError> {
    if !allow_empty && content.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Message content cannot be empty or only whitespace".to_string(),
        ));
    }

    if content.chars().count() > MAX_MESSAGE_CONTENT_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Message content cannot exceed {MAX_MESSAGE_CONTENT_LENGTH} characters"
        )));
    }

    Ok(())
}

/// 絵文字のバリデーションと正規化:
/// 1. ショートコード形式 (:thumbsup:, :+1:, :party_parrot:) -> 標準ショートコードは fully-qualified な絵文字に解決、その他は小文字ショートコードとして保持
/// 2. Emoji Kitchen 形式 (kitchen:1f600_1f431, kitchen:😀_🐱) -> 実在する絵文字 2 要素のみを受け付け、コードポイント順にソートして一意化
/// 3. Unicode 絵文字: emojis クレートを用いて emoji-test.txt 準拠の fully-qualified な形に正規化
pub fn normalize_and_validate_emoji(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Emoji cannot be empty".to_string()));
    }
    if trimmed.len() > 256 {
        return Err(AppError::BadRequest(
            "Emoji is too long (max 256 bytes)".to_string(),
        ));
    }

    // 1. ショートコード形式
    if trimmed.starts_with(':') && trimmed.ends_with(':') && trimmed.len() >= 3 {
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.len() <= 64
            && inner.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' || c == '+'
            })
        {
            if let Some(emoji) = emojis::get_by_shortcode(inner) {
                return Ok(emoji.as_str().to_string());
            }
            return Ok(trimmed.to_ascii_lowercase());
        }
    }

    // 2. Emoji Kitchen 形式 (例: kitchen:1f600_1f431, kitchen:😀_🐱)
    // 必ずアンダースコア区切りの 2 要素であり、両方が実在する絵文字に解決できる必要がある
    if let Some(rest) = trimmed.strip_prefix("kitchen:") {
        let parts: Vec<&str> = rest.split('_').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(AppError::BadRequest(
                "Emoji Kitchen must consist of exactly 2 emojis separated by an underscore (e.g. kitchen:1f600_1f431)"
                    .to_string(),
            ));
        }
        let part1_hex = parse_emoji_or_hex_codepoint(parts[0])?;
        let part2_hex = parse_emoji_or_hex_codepoint(parts[1])?;
        let (first, second) = if part1_hex <= part2_hex {
            (part1_hex, part2_hex)
        } else {
            (part2_hex, part1_hex)
        };
        return Ok(format!("kitchen:{first}_{second}"));
    }

    // 3. Unicode 絵文字 (fully-qualified 照合)
    if let Some(emoji) = resolve_unicode_emoji(trimmed) {
        return Ok(emoji.as_str().to_string());
    }

    Err(AppError::BadRequest(
        "Emoji must be a valid Unicode emoji, shortcode (e.g. :thumbsup:), or Emoji Kitchen format"
            .to_string(),
    ))
}

/// VS15 (\u{FE0E}) および VS16 (\u{FE0F}) を除去した文字列から canonical な絵文字へのマッピングテーブル。
/// 起動時/初回アクセス時に 1 回だけ構築され、以降は O(1) でルックアップ可能（全件走査 DoS の防止）。
static STRIPPED_EMOJI_MAP: LazyLock<HashMap<String, &'static emojis::Emoji>> =
    LazyLock::new(|| {
        let mut map = HashMap::new();
        for emoji in emojis::iter() {
            let stripped: String = emoji
                .as_str()
                .chars()
                .filter(|&c| c != '\u{FE0E}' && c != '\u{FE0F}')
                .collect();
            // 最初の fully-qualified な canonical 絵文字を登録
            map.entry(stripped).or_insert(emoji);
        }
        map
    });

/// Unicode 文字列から実在する canonical 絵文字 (&'static emojis::Emoji) を解決する
fn resolve_unicode_emoji(input: &str) -> Option<&'static emojis::Emoji> {
    if let Some(emoji) = emojis::get(input) {
        return Some(emoji);
    }

    // VS16 (\u{FE0F}) 付加によるフォールバック (例: ❤ -> ❤️)
    let candidate_with_vs16 = format!("{input}\u{FE0F}");
    if let Some(emoji) = emojis::get(&candidate_with_vs16) {
        return Some(emoji);
    }

    // キーキャップで VS16 が抜けている場合 (例: "1\u{20E3}" -> "1\u{FE0F}\u{20E3}")
    if input.ends_with('\u{20E3}') && !input.contains('\u{FE0F}') {
        let keycap_cand = format!(
            "{}\u{FE0F}\u{20E3}",
            &input[..input.len() - '\u{20E3}'.len_utf8()]
        );
        if let Some(emoji) = emojis::get(&keycap_cand) {
            return Some(emoji);
        }
    }

    // 事前構築されたマップを用いて VS15/VS16 を無視した canonical 絵文字を O(1) で検索
    let stripped_input: String = input
        .chars()
        .filter(|&c| c != '\u{FE0E}' && c != '\u{FE0F}')
        .collect();
    if stripped_input.is_empty() {
        return None;
    }
    STRIPPED_EMOJI_MAP.get(&stripped_input).copied()
}

/// Emoji Kitchen の 1 コンポーネントを検証し、正規化された hex 表記に解決する。
/// 絵文字そのもの、またはハイフン区切りの hex コードポイントを受け付け、実在する絵文字でなければエラーとする。
fn parse_emoji_or_hex_codepoint(part: &str) -> Result<String, AppError> {
    // 1. 絵文字そのものとして解決できる場合
    if let Some(emoji) = resolve_unicode_emoji(part) {
        let hexes: Vec<String> = emoji
            .as_str()
            .chars()
            .map(|c| format!("{:x}", c as u32))
            .collect();
        return Ok(hexes.join("-"));
    }

    // 2. hex 表記 (例: "1f600" や "1f1fa-1f1f8") のパースと実在検証
    let hex_tokens: Vec<&str> = part.split('-').collect();
    if !hex_tokens.is_empty()
        && hex_tokens.iter().all(|t| {
            !t.is_empty() && (1..=8).contains(&t.len()) && t.chars().all(|c| c.is_ascii_hexdigit())
        })
    {
        let mut chars = String::new();
        for token in hex_tokens {
            if let Ok(cp) = u32::from_str_radix(token, 16) {
                if let Some(c) = char::from_u32(cp) {
                    chars.push(c);
                } else {
                    return Err(AppError::BadRequest(format!(
                        "Invalid Unicode codepoint in Emoji Kitchen component: {part}"
                    )));
                }
            } else {
                return Err(AppError::BadRequest(format!(
                    "Invalid hex in Emoji Kitchen component: {part}"
                )));
            }
        }

        if let Some(emoji) = resolve_unicode_emoji(&chars) {
            let hexes: Vec<String> = emoji
                .as_str()
                .chars()
                .map(|c| format!("{:x}", c as u32))
                .collect();
            return Ok(hexes.join("-"));
        }
    }

    Err(AppError::BadRequest(format!(
        "Emoji Kitchen component '{part}' is not a valid emoji or emoji codepoint"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_and_validate_emoji() {
        // Unicode 絵文字
        assert_eq!(
            normalize_and_validate_emoji("👍").unwrap(),
            "👍".to_string()
        );
        // VS16 付きのハートと付いていないハートが同じ文字列に正規化されること
        let with_vs16 = "❤️"; // \u{2764}\u{FE0F}
        let without_vs16 = "❤"; // \u{2764}
        assert_eq!(
            normalize_and_validate_emoji(with_vs16).unwrap(),
            normalize_and_validate_emoji(without_vs16).unwrap()
        );

        // キーキャップや記号絵文字
        assert!(normalize_and_validate_emoji("1️⃣").is_ok());
        assert!(normalize_and_validate_emoji("©️").is_ok());

        // スキントーン修飾子付き絵文字
        assert!(normalize_and_validate_emoji("👍🏽").is_ok());

        // ショートコード (標準絵文字は絵文字に解決、カスタム絵文字は保持)
        assert_eq!(
            normalize_and_validate_emoji(":thumbsup:").unwrap(),
            "👍".to_string()
        );
        assert_eq!(
            normalize_and_validate_emoji(":+1:").unwrap(),
            "👍".to_string()
        );
        assert_eq!(
            normalize_and_validate_emoji(":party_parrot:").unwrap(),
            ":party_parrot:".to_string()
        );

        // Emoji Kitchen (ソートされて一意化されること)
        assert_eq!(
            normalize_and_validate_emoji("kitchen:1f600_1f431").unwrap(),
            "kitchen:1f431_1f600".to_string()
        );
        assert_eq!(
            normalize_and_validate_emoji("kitchen:1f431_1f600").unwrap(),
            "kitchen:1f431_1f600".to_string()
        );
        assert_eq!(
            normalize_and_validate_emoji("kitchen:😀_🐱").unwrap(),
            "kitchen:1f431_1f600".to_string()
        );

        // 不正な Emoji Kitchen (2要素でない、実在しない絵文字)
        assert!(normalize_and_validate_emoji("kitchen:hello").is_err());
        assert!(normalize_and_validate_emoji("kitchen:a_b_c").is_err());
        assert!(normalize_and_validate_emoji("kitchen:0_1").is_err());
        assert!(normalize_and_validate_emoji("kitchen:hello_world").is_err());
        assert!(normalize_and_validate_emoji("kitchen:_1f600").is_err());
        assert!(normalize_and_validate_emoji("kitchen:1f600_").is_err());

        // 不正な値
        assert!(normalize_and_validate_emoji("").is_err());
        assert!(normalize_and_validate_emoji("   ").is_err());
        assert!(normalize_and_validate_emoji("a").is_err());
        assert!(normalize_and_validate_emoji("hello").is_err());
        assert!(normalize_and_validate_emoji("123").is_err());
        assert!(normalize_and_validate_emoji("■").is_err());
        assert!(normalize_and_validate_emoji(":invalid with spaces:").is_err());
    }

    #[test]
    fn test_validate_channel_name() {
        assert!(validate_channel_name("general-chat").is_ok());
        assert!(validate_channel_name("dev-123").is_ok());

        for name in [
            "ざつだん",
            "チャンネル",
            "開発相談",
            "日々のメモ",
            "サーバー-開発-123",
            "開発_相談",
            "dev_123",
            "ﾁｬﾝﾈﾙ",
            "か\u{3099}",
            "𠮷野家",
            "General",
            "équipe",
            "فريق",
            "cafe\u{301}",
        ] {
            assert!(validate_channel_name(name).is_ok(), "{name}");
        }
        for character in ["あ", "𠮷"] {
            assert!(validate_channel_name(&character.repeat(80)).is_ok());
            assert!(validate_channel_name(&character.repeat(81)).is_err());
        }
        for name in [
            "",
            " ",
            "　",
            "\t",
            "hello world",
            "開発・相談",
            "開発😀",
            "#general",
            "test@",
            "開発/相談",
            "開発\\相談",
            "開発.相談",
            "開発:相談",
            "開発—相談",
            "開発−相談",
            "開発\n相談",
            "開発\r相談",
            "開発\0相談",
            "開発\u{2028}相談",
            "開発\u{2029}相談",
            "開発\u{200b}相談",
            "dm:00000000-0000-0000-0000-000000000001:00000000-0000-0000-0000-000000000002",
        ] {
            assert!(validate_channel_name(name).is_err(), "{name:?}");
        }
        // 予約語
        assert!(validate_channel_name("general").is_err());
        assert!(validate_channel_name("all").is_err());
        assert!(validate_channel_name("here").is_err());
        // 80文字超
        let too_long = "a".repeat(81);
        assert!(validate_channel_name(&too_long).is_err());
        let exact = "a".repeat(80);
        assert!(validate_channel_name(&exact).is_ok());
    }

    #[test]
    fn test_validate_channel_description() {
        assert!(validate_channel_description(None).is_ok());
        assert!(validate_channel_description(Some("")).is_ok());
        assert!(validate_channel_description(Some("A normal description")).is_ok());

        let too_long = "a".repeat(251);
        assert!(validate_channel_description(Some(&too_long)).is_err());
        let exact = "a".repeat(250);
        assert!(validate_channel_description(Some(&exact)).is_ok());
    }

    #[test]
    fn test_validate_user_name() {
        assert!(validate_user_name("a").is_ok());
        assert!(validate_user_name("john.doe").is_ok());
        assert!(validate_user_name("alice_123").is_ok());
        assert!(validate_user_name("bob-smith").is_ok());
        assert!(validate_user_name("0.-_").is_ok());
        assert!(validate_user_name("general").is_ok());

        let exact = "a".repeat(MAX_USER_NAME_LENGTH);
        assert!(validate_user_name(&exact).is_ok());
        let too_long = "a".repeat(MAX_USER_NAME_LENGTH + 1);
        assert!(validate_user_name(&too_long).is_err());

        assert!(validate_user_name("").is_err());
        assert!(validate_user_name("John").is_err());
        assert!(validate_user_name("user name").is_err());
        assert!(validate_user_name("user@example").is_err());
        assert!(validate_user_name("user+tag").is_err());
        assert!(validate_user_name("user/name").is_err());
        assert!(validate_user_name("あ").is_err());
        assert!(validate_user_name("Ａ").is_err());
        assert!(validate_user_name("user😀").is_err());

        for reserved in ["all", "channel", "everyone", "here", "slackbot", "admin"] {
            assert!(
                validate_user_name(reserved).is_err(),
                "{reserved} must be reserved"
            );
        }

        assert!(validate_user_name("admin1").is_ok());
        assert!(validate_user_name("channel.").is_ok());
    }

    #[test]
    fn test_validate_display_name() {
        assert!(validate_display_name("Alice").is_ok());
        assert!(validate_display_name("田中 太郎").is_ok());
        assert!(validate_display_name("  Bob  ").is_ok());

        assert!(validate_display_name("").is_err());
        assert!(validate_display_name("   ").is_err());
        let too_long = "a".repeat(81);
        assert!(validate_display_name(&too_long).is_err());
        // 前後に空白があってもトリム後が80文字ならOK
        let exact_with_spaces = format!("  {}  ", "a".repeat(80));
        assert!(validate_display_name(&exact_with_spaces).is_ok());
        // トリム後が81文字ならNG
        let too_long_with_spaces = format!("  {}  ", "a".repeat(81));
        assert!(validate_display_name(&too_long_with_spaces).is_err());
    }

    #[test]
    fn test_validate_message_content() {
        assert!(validate_message_content("Hello!").is_ok());
        assert!(validate_message_content("Line 1\nLine 2").is_ok());

        assert!(validate_message_content("").is_err());
        assert!(validate_message_content("   \n\t ").is_err());

        let too_long = "a".repeat(40_001);
        assert!(validate_message_content(&too_long).is_err());
        let exact = "a".repeat(40_000);
        assert!(validate_message_content(&exact).is_ok());

        // allow_empty = true (添付ファイルありの場合など)
        assert!(validate_message_content_with_options("", true).is_ok());
        assert!(validate_message_content_with_options("   \n\t ", true).is_ok());
        assert!(validate_message_content_with_options(&exact, true).is_ok());
        assert!(validate_message_content_with_options(&too_long, true).is_err());
    }
}
