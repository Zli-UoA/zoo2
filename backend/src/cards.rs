use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// 旧形式のカード（MessageCard）。旧形式の App API 入力の検証にだけ使う。
/// 保存は部品の木（`crate::components`）にする。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageCard {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<CardField>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardField {
    pub title: String,
    pub value: String,
    #[serde(default)]
    pub short: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub code: bool,
}
impl MessageCard {
    pub fn encode(&self) -> Result<String, AppError> {
        if self.fields.len() > 10
            || self.fields.iter().any(|field| {
                field.title.chars().count() > 100
                    || field.value.chars().count() > 1000
                    || field.url.chars().count() > 2000
                    || (!field.url.is_empty()
                        && !reqwest::Url::parse(&field.url)
                            .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some()))
            })
        {
            return Err(AppError::BadRequest("Invalid card fields".into()));
        }
        if !self.color.is_empty() && normalize_color(&self.color).is_none() {
            return Err(AppError::BadRequest("Invalid card color".into()));
        }
        if self.title.trim().is_empty()
            || self.title.chars().count() > 500
            || self.description.chars().count() > 4000
            || [
                &self.url,
                &self.source,
                &self.context,
                &self.author,
                &self.badge,
            ]
            .iter()
            .any(|s| s.chars().count() > 2000)
        {
            return Err(AppError::BadRequest("Invalid card fields or size".into()));
        }
        if !self.url.is_empty()
            && !reqwest::Url::parse(&self.url)
                .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
        {
            return Err(AppError::BadRequest("Card URL must use HTTPS".into()));
        }
        serde_json::to_string(self).map_err(|e| AppError::Internal(e.into()))
    }
}

pub fn normalize_color(value: &str) -> Option<String> {
    match value {
        "good" => Some("#2eb886".into()),
        "warning" => Some("#daa038".into()),
        "danger" => Some("#a30200".into()),
        _ => {
            let hex = value.strip_prefix('#').unwrap_or(value);
            (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| format!("#{hex}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_minimal_service_independent_cards() {
        let card: MessageCard = serde_json::from_str(r#"{"title":"Deployment complete"}"#).unwrap();
        assert!(card.encode().unwrap().contains("Deployment complete"));
        assert!(card.source.is_empty());
    }
    #[test]
    fn rejects_invalid_cards_and_unsafe_links() {
        for payload in [
            r#"{"title":""}"#,
            r#"{"title":"test","url":"javascript:alert(1)"}"#,
            r#"{"title":"test","url":"http://example.com"}"#,
        ] {
            assert!(
                serde_json::from_str::<MessageCard>(payload)
                    .unwrap()
                    .encode()
                    .is_err()
            );
        }
        assert!(
            serde_json::from_str::<MessageCard>(r#"{"title":"test","description":42}"#).is_err()
        );
        let card: MessageCard =
            serde_json::from_value(serde_json::json!({"title":"x".repeat(501)})).unwrap();
        assert!(card.encode().is_err());
    }
}
