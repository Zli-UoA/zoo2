//! Read-only Asterisk realtime projection. No client authentication or public SIP endpoint.
use crate::{apps::query, error::AppError, state::AppState};
use axum::{
    Form, Router,
    extract::{Path, State},
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::post,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
async fn one(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<Value, AppError> {
    Ok(query(tx, sql, params)
        .await?
        .into_iter()
        .next()
        .unwrap_or(Value::Null))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/internal/pbx/{table}/{operation}", post(lookup))
}
fn authorized(headers: &HeaderMap, secret: Option<&str>) -> bool {
    let Some(secret) = secret else { return false };
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    // Fixed-length MAC verification avoids comparing the bearer with an early-exit string equality.
    let mut expected =
        <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).expect("HMAC key");
    expected.update(b"zoo-pbx-config");
    let mut received =
        <Hmac<Sha256> as KeyInit>::new_from_slice(token.as_bytes()).expect("HMAC key");
    received.update(b"zoo-pbx-config");
    received
        .verify_slice(&expected.finalize().into_bytes())
        .is_ok()
}
fn encoded(row: &[(&str, String)]) -> String {
    let mut url = reqwest::Url::parse("http://localhost/").unwrap();
    let mut form = url.query_pairs_mut();
    for (key, value) in row {
        form.append_pair(key, value);
    }
    drop(form);
    url.query().unwrap_or("").to_owned()
}
fn endpoint_id(id: &str) -> Option<(&str, &str)> {
    let (kind, id) = id.split_once('-')?;
    if !matches!(kind, "line" | "home" | "phone" | "auth") || uuid::Uuid::parse_str(id).is_err() {
        return None;
    }
    Some((kind, id))
}
async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((table, operation)): Path<(String, String)>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    if !authorized(
        &headers,
        state.config.phone_gateway.pbx_config_secret.as_deref(),
    ) {
        return Err(AppError::Forbidden("PBX authentication required".into()));
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    if !matches!(
        table.as_str(),
        "endpoints" | "auths" | "aors" | "extensions"
    ) || !matches!(operation.as_str(), "single" | "multi" | "require")
    {
        return Err(AppError::NotFound("Unknown PBX projection".into()));
    }
    if operation == "require" {
        return Ok((response_headers, "0".to_owned()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let result = if table == "extensions" {
        let context = fields.get("context").map(String::as_str).unwrap_or("");
        let (home, id) = if let Some(id) = context.strip_prefix("zoo-home-") {
            (true, id)
        } else {
            (false, context.strip_prefix("zoo-line-").unwrap_or(""))
        };
        if uuid::Uuid::parse_str(id).is_err() {
            return Ok((response_headers, String::new()));
        }
        let line=one(&mut tx,"SELECT json_build_object('id',l.id)::text FROM sip_lines l JOIN users u ON u.id=l.created_by WHERE l.id=$1 AND NOT u.is_deactivated", &[id]).await?;
        if line.is_null() {
            return Ok((response_headers, String::new()));
        }
        let exten = fields.get("exten").map(String::as_str).unwrap_or("");
        let priority = fields.get("priority").map(String::as_str).unwrap_or("1");
        let data = if home && exten == "s" {
            Some(format!("${{PJSIP_DIAL_CONTACTS(line-{id})}},60"))
        } else if !home {
            let phone=one(&mut tx,"SELECT json_build_object('id',id)::text FROM sip_phones WHERE line_id=$1 AND extension=$2 LIMIT 1", &[id,exten]).await?;
            phone["id"]
                .as_str()
                .map(|id| format!("${{PJSIP_DIAL_CONTACTS(phone-{id})}},30"))
                .or_else(|| {
                    let digits = exten.strip_prefix('+').unwrap_or(exten);
                    (!digits.is_empty()
                        && digits.len() <= 32
                        && digits.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| format!("PJSIP/${{FILTER(0-9+,${{EXTEN}})}}@home-{id},60"))
                })
        } else {
            None
        };
        match (priority, data) {
            ("1", Some(data)) => encoded(&[
                ("context", context.into()),
                ("exten", exten.into()),
                ("priority", "1".into()),
                ("app", "Dial".into()),
                ("appdata", data),
            ]),
            ("2", Some(_)) => encoded(&[
                ("context", context.into()),
                ("exten", exten.into()),
                ("priority", "2".into()),
                ("app", "Hangup".into()),
                ("appdata", String::new()),
            ]),
            _ => String::new(),
        }
    } else {
        let Some(id) = fields.get("id") else {
            return Ok((response_headers, String::new()));
        };
        let Some((kind, uuid)) = endpoint_id(id) else {
            return Ok((response_headers, String::new()));
        };
        let line = if kind == "phone" {
            one(&mut tx,"SELECT json_build_object('id',l.id,'sip_domain',l.sip_domain,'username',l.username,'password_ciphertext',l.password_ciphertext)::text FROM sip_phones p JOIN sip_lines l ON l.id=p.line_id JOIN users u ON u.id=l.created_by WHERE p.id=$1 AND NOT u.is_deactivated", &[uuid]).await?
        } else {
            one(&mut tx,"SELECT json_build_object('id',l.id,'sip_domain',l.sip_domain,'username',l.username,'password_ciphertext',l.password_ciphertext)::text FROM sip_lines l JOIN users u ON u.id=l.created_by WHERE l.id=$1 AND NOT u.is_deactivated", &[uuid]).await?
        };
        if line.is_null() {
            return Ok((response_headers, String::new()));
        }
        let line_id = line["id"].as_str().unwrap();
        match table.as_str() {
            "endpoints" if kind != "auth" => encoded(&[
                ("id", id.clone()),
                ("transport", "transport-tcp".into()),
                ("aors", id.clone()),
                ("auth", format!("auth-{line_id}")),
                (
                    "context",
                    if kind == "home" {
                        "zoo-home"
                    } else {
                        "zoo-call"
                    }
                    .into(),
                ),
                ("set_var", format!("ZOO_LINE_ID={line_id}")),
                ("disallow", "all".into()),
                ("allow", "ulaw,alaw".into()),
                ("direct_media", "no".into()),
                ("force_rport", "yes".into()),
                ("rewrite_contact", "yes".into()),
                ("rtp_symmetric", "yes".into()),
                ("identify_by", "username".into()),
            ]),
            "aors" if kind != "auth" => encoded(&[
                ("id", id.clone()),
                (
                    "max_contacts",
                    if kind == "line" { "128" } else { "1" }.into(),
                ),
                (
                    "remove_existing",
                    if kind == "line" { "no" } else { "yes" }.into(),
                ),
                ("qualify_frequency", "0".into()),
                ("default_expiration", "60".into()),
                ("maximum_expiration", "120".into()),
            ]),
            "auths" if kind == "auth" => {
                let password = super::sip::crypt(
                    &super::sip::key(&state)?,
                    line_id,
                    line["password_ciphertext"].as_str().unwrap(),
                    true,
                )?;
                encoded(&[
                    ("id", id.clone()),
                    ("auth_type", "userpass".into()),
                    ("username", line["username"].as_str().unwrap().into()),
                    ("realm", line["sip_domain"].as_str().unwrap().into()),
                    ("password", password),
                ])
            }
            _ => String::new(),
        }
    };
    Ok((response_headers, result))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_and_projection_bounds() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer dummy".parse().unwrap());
        assert!(authorized(&headers, Some("dummy")));
        assert!(!authorized(&headers, Some("other")));
        assert!(!authorized(&headers, None));
        assert!(endpoint_id("line-550e8400-e29b-41d4-a716-446655440000").is_some());
        assert!(endpoint_id("line-../auths").is_none());
        assert_eq!(
            encoded(&[("password", "a&b=c\n".into())]),
            "password=a%26b%3Dc%0A"
        );
    }
}
