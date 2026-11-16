//! Private SIP directory. Asterisk can read this directory through the server-only realtime adapter.
use crate::{
    apps::{one, query},
    auth::{AuthUser, rbac::is_workspace_admin},
    error::AppError,
    state::AppState,
};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/sip/lines", get(list).post(create))
        .route("/api/sip/lines/{id}", post(update).delete(remove))
        .route("/api/sip/lines/{id}/connect", post(connect))
        .route("/api/sip/lines/{id}/authorize", post(authorize))
}

pub(crate) fn parse_key(value: Option<&str>) -> Result<Option<[u8; 32]>, String> {
    value
        .map(|v| {
            STANDARD
                .decode(v)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| "SIP_ENCRYPTION_KEY must be base64-encoded 32 bytes".to_owned())
        })
        .transpose()
}
pub(crate) fn key(state: &AppState) -> Result<[u8; 32], AppError> {
    parse_key(state.config.sip_encryption_key.as_deref())
        .map_err(|_| AppError::ServiceUnavailable("SIP configuration is invalid".into()))?
        .ok_or_else(|| AppError::ServiceUnavailable("SIP is not enabled".into()))
}
pub(crate) fn crypt(
    key: &[u8; 32],
    id: &str,
    value: &str,
    decrypt: bool,
) -> Result<String, AppError> {
    let invalid = || AppError::Internal(anyhow::anyhow!("SIP credential encryption failed"));
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| invalid())?;
    if decrypt {
        let data = STANDARD.decode(value).map_err(|_| invalid())?;
        if data.len() < 28 {
            return Err(invalid());
        }
        let plain = cipher
            .decrypt(
                Nonce::from_slice(&data[..12]),
                Payload {
                    msg: &data[12..],
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        String::from_utf8(plain).map_err(|_| invalid())
    } else {
        use aes_gcm::aead::{AeadCore, OsRng};
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let encrypted = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: value.as_bytes(),
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        let mut data = nonce.to_vec();
        data.extend(encrypted);
        Ok(STANDARD.encode(data))
    }
}
pub(crate) fn private(value: Value) -> (HeaderMap, Json<Value>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    (headers, Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Phone {
    #[serde(default)]
    id: Option<String>,
    name: String,
    extension: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineInput {
    name: String,
    shared: bool,
    #[serde(default = "legacy_path")]
    websocket_path: String,
    sip_domain: String,
    username: String,
    /// Missing/empty on update preserves the existing password. Never returned in a directory.
    password: Option<String>,
    #[serde(default)]
    phone_number: String,
    #[serde(default)]
    member_ids: Vec<String>,
    #[serde(default)]
    phones: Vec<Phone>,
}
fn legacy_path() -> String {
    "/sip/ws".into()
}
fn safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn number(s: &str) -> bool {
    !s.is_empty()
        && s.strip_prefix('+').unwrap_or(s).len() <= 32
        && s.strip_prefix('+')
            .unwrap_or(s)
            .bytes()
            .all(|b| b.is_ascii_digit())
        && s != "+"
}
impl LineInput {
    fn validate(&self) -> Result<(), AppError> {
        let path = &self.websocket_path;
        if self.name.trim().is_empty()
            || self.name.chars().count() > 100
            || !path.starts_with("/sip/")
            || path.len() > 200
            || path.contains("..")
            || !path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b))
            || !safe_token(&self.sip_domain)
            || !safe_token(&self.username)
            || (!self.phone_number.is_empty() && !number(&self.phone_number))
            || self.password.as_ref().is_some_and(|p| p.len() > 1024)
            || self.member_ids.len() > 100
            || self.phones.len() > 100
            || self.phones.iter().any(|p| {
                p.name.trim().is_empty() || p.name.chars().count() > 100 || !number(&p.extension)
            })
            || (!self.shared && !self.member_ids.is_empty())
        {
            return Err(AppError::BadRequest("Invalid SIP line settings".into()));
        }
        Ok(())
    }
}
async fn active(tx: &mut toasty::Transaction<'_>, user: &AuthUser) -> Result<(), AppError> {
    let row=one(tx,"SELECT json_build_object('valid',EXISTS(SELECT 1 FROM users WHERE id=$1 AND NOT is_bot AND NOT is_deactivated))::text", &[&user.id]).await?;
    if row["valid"] != true {
        return Err(AppError::Forbidden("Active user required".into()));
    }
    Ok(())
}
async fn admin(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    user: &AuthUser,
) -> Result<bool, AppError> {
    is_workspace_admin(tx, &user.id, state.config.dev_auth_enabled).await
}
fn manageable(row: &Value, user: &AuthUser, is_admin: bool) -> bool {
    row["owner_user_id"] == user.id || (row["owner_user_id"].is_null() && is_admin)
}
async fn locked(tx: &mut toasty::Transaction<'_>, id: &str) -> Result<Value, AppError> {
    let row=one(tx,"SELECT json_build_object('id',id,'owner_user_id',owner_user_id,'password_ciphertext',password_ciphertext)::text FROM sip_lines WHERE id=$1 FOR UPDATE",&[id]).await?;
    if row.is_null() {
        return Err(AppError::NotFound("SIP line not found".into()));
    }
    Ok(row)
}
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    if state.config.sip_encryption_key.is_none() {
        return Ok(private(json!({"enabled":false,"lines":[]})));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    active(&mut tx, &user).await?;
    let is_admin = admin(&mut tx, &state, &user).await?;
    let lines=query(&mut tx,"SELECT json_build_object('id',l.id,'name',l.name,'owner_user_id',l.owner_user_id,'shared',l.owner_user_id IS NULL,'websocket_path',l.websocket_path,'sip_domain',l.sip_domain,'username',l.username,'phone_number',l.phone_number,'can_manage',(l.owner_user_id=$1 OR (l.owner_user_id IS NULL AND $2='true')) IS TRUE,'can_connect',(l.owner_user_id=$1 OR EXISTS(SELECT 1 FROM sip_line_members m WHERE m.line_id=l.id AND m.user_id=$1)) IS TRUE,'member_ids',CASE WHEN l.owner_user_id IS NULL AND $2='true' THEN COALESCE((SELECT json_agg(user_id ORDER BY user_id) FROM (SELECT user_id FROM sip_line_members WHERE line_id=l.id ORDER BY user_id LIMIT 100) m),'[]'::json) ELSE '[]'::json END,'phones',COALESCE((SELECT json_agg(json_build_object('id',p.id,'name',p.name,'extension',p.extension) ORDER BY p.name) FROM (SELECT id,name,extension FROM sip_phones WHERE line_id=l.id ORDER BY extension LIMIT 100) p),'[]'::json))::text FROM sip_lines l WHERE l.id IN (SELECT id FROM sip_lines WHERE owner_user_id=$1 UNION SELECT line_id FROM sip_line_members WHERE user_id=$1 UNION SELECT id FROM sip_lines WHERE owner_user_id IS NULL AND $2='true') ORDER BY l.name,l.id",&[&user.id,if is_admin{"true"}else{"false"}]).await?;
    Ok(private(json!({"enabled":true,"lines":lines})))
}
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<LineInput>,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    save(state, user, None, body).await
}
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<LineInput>,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    save(state, user, Some(id), body).await
}
async fn save(
    state: AppState,
    user: AuthUser,
    existing: Option<String>,
    body: LineInput,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    let encryption_key = key(&state)?;
    body.validate()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    active(&mut tx, &user).await?;
    let is_admin = admin(&mut tx, &state, &user).await?;
    let id = existing
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Use the same user → line → session lock order as gateway reservations.
    toasty::sql::statement("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(user.id.clone())
        .exec(&mut tx)
        .await?;
    let old = if existing.is_some() {
        let row = locked(&mut tx, &id).await?;
        if !manageable(&row, &user, is_admin) {
            return Err(AppError::NotFound("SIP line not found".into()));
        }
        if row["owner_user_id"].is_null() != body.shared {
            return Err(AppError::BadRequest(
                "SIP line ownership cannot be changed".into(),
            ));
        }
        Some(row)
    } else {
        None
    };
    if body.shared && !is_admin {
        return Err(AppError::Forbidden(
            "Administrator required for shared lines".into(),
        ));
    }
    let password = match body.password.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => crypt(&encryption_key, &id, p, false)?,
        None => old
            .as_ref()
            .and_then(|o| o["password_ciphertext"].as_str())
            .ok_or_else(|| AppError::BadRequest("SIP password required".into()))?
            .to_owned(),
    };
    for member in &body.member_ids {
        let valid=one(&mut tx,"SELECT json_build_object('valid',EXISTS(SELECT 1 FROM users WHERE id=$1 AND NOT is_bot AND NOT is_deactivated))::text",&[member]).await?;
        if valid["valid"] != true {
            return Err(AppError::BadRequest("SIP member is unavailable".into()));
        }
    }
    if old.is_none() {
        let count = one(
            &mut tx,
            "SELECT json_build_object('count',count(*))::text FROM sip_lines WHERE created_by=$1",
            &[&user.id],
        )
        .await?;
        if count["count"].as_i64().unwrap_or(0) >= 50 {
            return Err(AppError::BadRequest("SIP line limit reached".into()));
        }
        toasty::sql::statement("INSERT INTO sip_lines(id,name,owner_user_id,websocket_path,sip_domain,username,password_ciphertext,phone_number,created_by) VALUES($1,$2,CASE WHEN $3 THEN NULL ELSE $4 END,$5,$6,$7,$8,$9,$4)")
 .bind(id.clone()).bind(body.name.trim()).bind(body.shared).bind(user.id.clone()).bind(body.websocket_path.clone()).bind(body.sip_domain.clone()).bind(body.username.clone()).bind(password).bind(body.phone_number.clone()).exec(&mut tx).await?;
    } else {
        toasty::sql::statement("UPDATE sip_lines SET name=$2,websocket_path=$3,sip_domain=$4,username=$5,password_ciphertext=$6,phone_number=$7,updated_at=now() WHERE id=$1")
 .bind(id.clone()).bind(body.name.trim()).bind(body.websocket_path).bind(body.sip_domain).bind(body.username).bind(password).bind(body.phone_number).exec(&mut tx).await?;
    }
    toasty::sql::statement("DELETE FROM sip_line_members WHERE line_id=$1")
        .bind(id.clone())
        .exec(&mut tx)
        .await?;
    for member in &body.member_ids {
        toasty::sql::statement(
            "INSERT INTO sip_line_members(line_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(id.clone())
        .bind(member.clone())
        .exec(&mut tx)
        .await?;
    }
    let existing_phones = query(
        &mut tx,
        "SELECT to_json(id)::text FROM sip_phones WHERE line_id=$1",
        &[&id],
    )
    .await?;
    let existing_ids: std::collections::HashSet<&str> =
        existing_phones.iter().filter_map(Value::as_str).collect();
    let mut phone_ids = std::collections::HashSet::new();
    for phone in &body.phones {
        if let Some(phone_id) = &phone.id {
            if !phone_ids.insert(phone_id.clone()) || !existing_ids.contains(phone_id.as_str()) {
                return Err(AppError::BadRequest("Invalid or duplicate phone ID".into()));
            }
        }
    }
    toasty::sql::statement("DELETE FROM sip_phones WHERE line_id=$1")
        .bind(id.clone())
        .exec(&mut tx)
        .await?;
    for p in body.phones {
        toasty::sql::statement(
            "INSERT INTO sip_phones(id,line_id,name,extension) VALUES($1,$2,$3,$4)",
        )
        .bind(p.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()))
        .bind(id.clone())
        .bind(p.name.trim())
        .bind(p.extension)
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;
    Ok(private(json!({"data":{"id":id},"txid":null})))
}
async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    key(&state)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    active(&mut tx, &user).await?;
    let is_admin = admin(&mut tx, &state, &user).await?;
    let row = locked(&mut tx, &id).await?;
    if !manageable(&row, &user, is_admin) {
        return Err(AppError::NotFound("SIP line not found".into()));
    }
    toasty::sql::statement("DELETE FROM sip_lines WHERE id=$1")
        .bind(id)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(private(json!({"data":{},"txid":null})))
}
async fn connect(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    let encryption_key = key(&state)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    active(&mut tx, &user).await?;
    let row=one(&mut tx,"SELECT json_build_object('id',id,'websocket_path',websocket_path,'sip_domain',sip_domain,'username',username,'password_ciphertext',password_ciphertext,'revision',updated_at::text)::text FROM sip_lines l WHERE id=$1 AND (owner_user_id=$2 OR (owner_user_id IS NULL AND EXISTS(SELECT 1 FROM sip_line_members WHERE line_id=l.id AND user_id=$2))) FOR SHARE OF l",&[&id,&user.id]).await?;
    if row.is_null() {
        return Err(AppError::NotFound("SIP line not found".into()));
    }
    let password = crypt(
        &encryption_key,
        &id,
        row["password_ciphertext"].as_str().unwrap_or_default(),
        true,
    )?;
    // Existing ICE configuration is private and sent only on explicit connection.
    let ice: Value = serde_json::from_str(&state.config.webrtc_ice_servers_json)
        .map_err(|_| AppError::ServiceUnavailable("Invalid ICE configuration".into()))?;
    let has_turn = ice.as_array().is_some_and(|servers| {
        servers.iter().any(|server| {
            let is_turn = |url: &str| url.starts_with("turn:") || url.starts_with("turns:");
            server["urls"].as_str().is_some_and(is_turn)
                || server["urls"]
                    .as_array()
                    .is_some_and(|urls| urls.iter().any(|u| u.as_str().is_some_and(is_turn)))
        })
    });
    if !ice.is_array() || (state.config.phone_gateway.relay_only() && !has_turn) {
        return Err(AppError::ServiceUnavailable(
            "SIP requires a TURN media relay".into(),
        ));
    }
    // Release the read lock before a separate reservation transaction or network I/O.
    tx.commit().await?;
    let connection =
        super::phone_sessions::connect(&state, &user, &id, &row, &password, ice).await?;
    Ok(private(json!({"data":connection,"txid":null})))
}

async fn authorize(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<(HeaderMap, Json<Value>), AppError> {
    key(&state)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    active(&mut tx, &user).await?;
    let row = one(&mut tx, "SELECT json_build_object('revision',updated_at::text)::text FROM sip_lines l WHERE id=$1 AND (owner_user_id=$2 OR (owner_user_id IS NULL AND EXISTS(SELECT 1 FROM sip_line_members WHERE line_id=l.id AND user_id=$2)))", &[&id,&user.id]).await?;
    Ok(private(json!({"data":row,"txid":null})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_randomized_and_bound_to_line() {
        let k = [7; 32];
        let c = crypt(&k, "a", "test-password", false).unwrap();
        assert_eq!(crypt(&k, "a", &c, true).unwrap(), "test-password");
        assert_ne!(c, crypt(&k, "a", "test-password", false).unwrap());
        assert!(crypt(&k, "b", &c, true).is_err());
        assert!(crypt(&[8; 32], "a", &c, true).is_err());
        assert!(crypt(&k, "a", "short", true).is_err());
    }
    #[test]
    fn key_validation() {
        assert!(parse_key(None).unwrap().is_none());
        assert!(parse_key(Some("secret")).is_err());
        assert_eq!(
            parse_key(Some(&STANDARD.encode([1; 32]))).unwrap(),
            Some([1; 32])
        );
    }
    #[test]
    fn administrator_cannot_manage_private_line() {
        let u = AuthUser {
            id: "bob".into(),
            user_name: "bob".into(),
            display_name: "Bob".into(),
        };
        assert!(!manageable(&json!({"owner_user_id":"alice"}), &u, true));
        assert!(manageable(&json!({"owner_user_id":null}), &u, true));
        assert!(!manageable(&json!({"owner_user_id":null}), &u, false));
    }
    #[test]
    fn rejects_cross_origin_and_sip_injection() {
        let mut b:LineInput=serde_json::from_value(json!({"name":"Home","shared":false,"websocket_path":"/sip/ws","sip_domain":"pbx.example","username":"alice","password":"dummy"})).unwrap();
        assert!(b.validate().is_ok());
        for p in [
            "//evil/sip/ws",
            "https://evil/ws",
            "/sip/../api/users",
            "/sip/ws?x=y",
        ] {
            b.websocket_path = p.into();
            assert!(b.validate().is_err());
        }
        b.websocket_path = "/sip/ws".into();
        b.username = "alice@evil".into();
        assert!(b.validate().is_err());
        assert!(!number("+"));
        assert!(!number("123;foo"));
    }
}
