use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// Service-independent card data. Only trusted integration routes may persist it.
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
}
impl MessageCard {
    pub fn encode(&self) -> Result<String, AppError> {
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
