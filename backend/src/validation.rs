use crate::error::AppError;

pub const MAX_CHANNEL_NAME_LENGTH: usize = 80;
pub const MAX_CHANNEL_DESCRIPTION_LENGTH: usize = 250;
pub const MAX_USER_NAME_LENGTH: usize = 80;
pub const MAX_DISPLAY_NAME_LENGTH: usize = 80;
pub const MAX_MESSAGE_CONTENT_LENGTH: usize = 40_000;

const RESERVED_CHANNEL_NAMES: &[&str] = &["all", "channel", "everyone", "here", "general"];
pub(crate) const RESERVED_USER_NAMES: &[&str] =
    &["all", "channel", "everyone", "here", "slackbot", "admin"];

/// チャンネル名のバリデーション (Slack 準拠):
/// - 1 文字以上 80 文字以下
/// - 半角英小文字、数字、ハイフン (`-`)、アンダースコア (`_`) のみ許可
/// - 予約語 (`all`, `channel`, `everyone`, `here`, `general`) の禁止
pub fn validate_channel_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "Channel name cannot be empty".to_string(),
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

    let is_valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');

    if !is_valid {
        return Err(AppError::BadRequest(
            "Channel name can only contain lowercase letters, numbers, hyphens, and underscores"
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
    let trimmed = content.trim();
    if trimmed.is_empty() {
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

/// 絵文字の正規化とバリデーション:
/// 1. ショートコード形式: `:thumbsup:`, `:+1:`, `:-1:` (`:[a-z0-9_+-]{1,64}:`)
/// 2. Emoji Kitchen 形式: `kitchen:<codepoints>`
/// 3. Unicode 絵文字: 単一または合成絵文字。VS15 (\u{FE0E}) / VS16 (\u{FE0F}) をストリップして一貫したバイナリに正規化。
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
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '_'
                    || c == '-'
                    || c == '+'
            })
        {
            return Ok(trimmed.to_ascii_lowercase());
        }
    }

    // 2. Emoji Kitchen 形式 (例: kitchen:1f600_1f431)
    if let Some(rest) = trimmed.strip_prefix("kitchen:") {
        if !rest.is_empty()
            && rest.chars().all(|c| {
                c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+'
            })
        {
            return Ok(format!("kitchen:{}", rest.to_ascii_lowercase()));
        }
    }

    // 3. Unicode 絵文字: VS15 (\u{FE0E}) と VS16 (\u{FE0F}) をストリップして正規化
    let normalized: String = trimmed
        .chars()
        .filter(|&c| c != '\u{FE0E}' && c != '\u{FE0F}')
        .collect();

    if normalized.is_empty() {
        return Err(AppError::BadRequest("Emoji is invalid".to_string()));
    }

    let is_valid_emoji = normalized.chars().all(is_emoji_codepoint);
    let has_actual_emoji = normalized.chars().any(is_primary_emoji_codepoint);

    if !is_valid_emoji || !has_actual_emoji {
        return Err(AppError::BadRequest(
            "Emoji must be a valid Unicode emoji, shortcode (e.g. :thumbsup:), or Emoji Kitchen format"
                .to_string(),
        ));
    }

    Ok(normalized)
}

fn is_emoji_codepoint(c: char) -> bool {
    is_primary_emoji_codepoint(c)
        || matches!(
            c,
            '\u{200D}' // ZWJ
            | '\u{1F3FB}'..='\u{1F3FF}' // Skin tone modifiers
        )
}

fn is_primary_emoji_codepoint(c: char) -> bool {
    matches!(
        c,
        '\u{1F000}'..='\u{1FAFF}'
            | '\u{2600}'..='\u{27BF}'
            | '\u{2300}'..='\u{23FF}'
            | '\u{2B50}'
            | '\u{2B55}'
            | '\u{203C}'
            | '\u{2049}'
            | '\u{2194}'..='\u{21AA}'
            | '\u{25A0}'..='\u{25FF}'
            | '\u{2934}'..='\u{2935}'
            | '\u{3030}'
            | '\u{303D}'
            | '\u{3297}'
            | '\u{3299}'
    )
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

        // スキントーン修飾子付き絵文字
        assert!(normalize_and_validate_emoji("👍🏽").is_ok());

        // ショートコード
        assert_eq!(
            normalize_and_validate_emoji(":thumbsup:").unwrap(),
            ":thumbsup:".to_string()
        );
        assert_eq!(
            normalize_and_validate_emoji(":+1:").unwrap(),
            ":+1:".to_string()
        );
        assert_eq!(
            normalize_and_validate_emoji(":-1:").unwrap(),
            ":-1:".to_string()
        );

        // Emoji Kitchen
        assert_eq!(
            normalize_and_validate_emoji("kitchen:1f600_1f431").unwrap(),
            "kitchen:1f600_1f431".to_string()
        );

        // 不正な値
        assert!(normalize_and_validate_emoji("").is_err());
        assert!(normalize_and_validate_emoji("   ").is_err());
        assert!(normalize_and_validate_emoji("a").is_err());
        assert!(normalize_and_validate_emoji("hello").is_err());
        assert!(normalize_and_validate_emoji("123").is_err());
        assert!(normalize_and_validate_emoji(":invalid with spaces:").is_err());
    }

    #[test]
    fn test_validate_channel_name() {
        assert!(validate_channel_name("general-chat").is_ok());
        assert!(validate_channel_name("dev_123").is_ok());

        // 空文字
        assert!(validate_channel_name("").is_err());
        // 大文字
        assert!(validate_channel_name("General").is_err());
        // スペース
        assert!(validate_channel_name("hello world").is_err());
        // 特殊記号
        assert!(validate_channel_name("#general").is_err());
        assert!(validate_channel_name("test@").is_err());
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
        assert!(validate_user_name("john.doe").is_ok());
        assert!(validate_user_name("alice_123").is_ok());
        assert!(validate_user_name("bob-smith").is_ok());

        assert!(validate_user_name("").is_err());
        assert!(validate_user_name("John").is_err());
        assert!(validate_user_name("user name").is_err());
        assert!(validate_user_name("slackbot").is_err());
        assert!(validate_user_name("admin").is_err());

        let too_long = "a".repeat(81);
        assert!(validate_user_name(&too_long).is_err());
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
    }
}
