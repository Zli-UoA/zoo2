use crate::error::AppError;

pub const MAX_CHANNEL_NAME_LENGTH: usize = 80;
pub const MAX_CHANNEL_DESCRIPTION_LENGTH: usize = 250;
pub const MAX_USER_NAME_LENGTH: usize = 80;
pub const MAX_DISPLAY_NAME_LENGTH: usize = 80;
pub const MAX_MESSAGE_CONTENT_LENGTH: usize = 40_000;

const RESERVED_CHANNEL_NAMES: &[&str] = &["all", "channel", "everyone", "here", "general"];
const RESERVED_USER_NAMES: &[&str] = &["all", "channel", "everyone", "here", "slackbot", "admin"];

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

    if name.chars().count() > MAX_DISPLAY_NAME_LENGTH {
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

#[cfg(test)]
mod tests {
    use super::*;

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
