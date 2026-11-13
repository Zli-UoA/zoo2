//! E2EE の Secret Storage（SSSS）と鍵スロットの API（#356）。仕様は `docs/e2ee.md` の「Secret Storage」
//! 「SSSS（秘密の暗号化）」「鍵スロット」。
//!
//! サーバーは暗号文を復号できない。形（base64 とバイト長）だけを確かめて保存する。

use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
};
use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::AppError,
    routes::{
        e2ee::{coded, exec, first, limit_exceeded, lock_active_user, lock_user},
        e2ee_cross_signing::present,
        e2ee_reauth::reauth,
        response::ApiResponse,
    },
    state::AppState,
};

/// SSSS 鍵の方式。
pub const SSSS_ALGORITHM: &str = "m.secret_storage.v1.aes-hmac-sha2";
/// 初回設定と作り直しで必須の秘密。
pub const REQUIRED_SECRETS: [&str; 4] = [
    "m.cross_signing.master",
    "m.cross_signing.self_signing",
    "m.cross_signing.user_signing",
    "m.megolm_backup.v1",
];
/// 失効のあとに置く、古い版のバックアップ鍵（省略できる）。
pub const PREVIOUS_BACKUP_SECRET: &str = "zoo.megolm_backup.previous";
/// 1 つの SSSS 鍵に置けるパスキーの鍵スロットの上限（解錠の `allowCredentials` の長さにもなる）。
pub const MAX_PASSKEY_SLOTS: usize = 20;
/// 1 つの秘密の暗号文（base64 の文字列）の長さの上限。
pub const MAX_SECRET_CIPHERTEXT_CHARS: usize = 64 * 1024;
/// WebAuthn の credential ID は最大 1023 バイト。base64url（パディングなし）で 1364 文字。
const MAX_CREDENTIAL_ID_CHARS: usize = 1364;
/// 鍵スロットのラベルの長さの上限（文字数）。
const MAX_LABEL_CHARS: usize = 255;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/e2ee/secret_storage", get(get_secret_storage))
        .route("/api/e2ee/secret_storage/keys", post(create_key))
        .route("/api/e2ee/secret_storage/secrets", post(put_secrets))
        .route("/api/e2ee/secret_storage/slots", post(add_slot))
        .route(
            "/api/e2ee/secret_storage/slots/{slot_id}",
            delete(delete_slot),
        )
        .route(
            "/api/e2ee/secret_storage/slots/{slot_id}/used",
            post(mark_slot_used),
        )
}

// ---------------------------------------------------------------------------
// 入力の検証

/// 標準の base64。パディングはあってもなくてもよい（docs/e2ee.md の「符号化と署名」）。
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

fn bad(field: &str, why: &str) -> AppError {
    AppError::BadRequest(format!("{field}: {why}"))
}

/// `value` が `len` バイト（`None` なら 1 バイト以上）の base64 か。
fn check_base64(field: &str, value: &str, len: Option<usize>) -> Result<(), AppError> {
    let bytes = BASE64
        .decode(value)
        .map_err(|_| bad(field, "must be base64"))?;
    match len {
        Some(len) if bytes.len() != len => Err(bad(field, &format!("must be {len} bytes"))),
        None if bytes.is_empty() => Err(bad(field, "must not be empty")),
        _ => Ok(()),
    }
}

/// 小文字でハイフン区切りの UUID（`key_id`、`slot_id`）。AAD に文字列のまま入るので、表記を固定する。
fn check_uuid(field: &str, value: &str) -> Result<(), AppError> {
    if Uuid::parse_str(value).is_ok_and(|u| u.hyphenated().to_string() == value) {
        Ok(())
    } else {
        Err(bad(field, "must be a lowercase hyphenated UUID"))
    }
}

#[derive(Deserialize)]
pub struct KeyDescription {
    algorithm: String,
    iv: String,
    mac: String,
}

impl KeyDescription {
    fn validate(&self) -> Result<(), AppError> {
        if self.algorithm != SSSS_ALGORITHM {
            return Err(bad("key.algorithm", "unsupported algorithm"));
        }
        check_base64("key.iv", &self.iv, Some(16))?;
        check_base64("key.mac", &self.mac, Some(32))
    }
}

#[derive(Deserialize)]
pub struct EncryptedSecret {
    iv: String,
    ciphertext: String,
    mac: String,
}

fn validate_secrets(
    secrets: &std::collections::BTreeMap<String, EncryptedSecret>,
) -> Result<(), AppError> {
    for (name, secret) in secrets {
        if !REQUIRED_SECRETS.contains(&name.as_str()) && name != PREVIOUS_BACKUP_SECRET {
            return Err(bad("secrets", &format!("unknown secret {name}")));
        }
        if secret.ciphertext.len() > MAX_SECRET_CIPHERTEXT_CHARS {
            return Err(limit_exceeded("secret_size"));
        }
        check_base64(&format!("{name}.iv"), &secret.iv, Some(16))?;
        check_base64(&format!("{name}.ciphertext"), &secret.ciphertext, None)?;
        check_base64(&format!("{name}.mac"), &secret.mac, Some(32))?;
    }
    Ok(())
}

fn secrets_json(secrets: &std::collections::BTreeMap<String, EncryptedSecret>) -> String {
    Value::Array(
        secrets
            .iter()
            .map(|(name, s)| json!({"name": name, "iv": s.iv, "ciphertext": s.ciphertext, "mac": s.mac}))
            .collect(),
    )
    .to_string()
}

#[derive(Deserialize, Clone)]
pub struct SlotInput {
    id: String,
    kind: String,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    prf_salt: Option<String>,
    nonce: String,
    ciphertext: String,
    #[serde(default)]
    label: String,
}

impl SlotInput {
    fn validate(&self) -> Result<(), AppError> {
        check_uuid("slot.id", &self.id)?;
        match (self.kind.as_str(), &self.credential_id, &self.prf_salt) {
            ("passkey", Some(credential_id), Some(prf_salt)) => {
                if credential_id.is_empty()
                    || credential_id.len() > MAX_CREDENTIAL_ID_CHARS
                    || !credential_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err(bad(
                        "slot.credential_id",
                        "must be unpadded base64url of at most 1023 bytes",
                    ));
                }
                check_base64("slot.prf_salt", prf_salt, Some(32))?;
            }
            ("recovery", None, None) => {}
            ("passkey", _, _) => {
                return Err(bad(
                    "slot",
                    "a passkey slot needs credential_id and prf_salt",
                ));
            }
            ("recovery", _, _) => {
                return Err(bad(
                    "slot",
                    "a recovery slot must not have credential_id or prf_salt",
                ));
            }
            _ => return Err(bad("slot.kind", "must be passkey or recovery")),
        }
        check_base64("slot.nonce", &self.nonce, Some(12))?;
        // SSSS 鍵の 32 バイトと、AES-GCM のタグの 16 バイト。
        check_base64("slot.ciphertext", &self.ciphertext, Some(48))?;
        if self.label.chars().count() > MAX_LABEL_CHARS {
            return Err(bad(
                "slot.label",
                &format!("must be at most {MAX_LABEL_CHARS} characters"),
            ));
        }
        Ok(())
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id, "kind": self.kind, "credential_id": self.credential_id,
            "prf_salt": self.prf_salt, "nonce": self.nonce, "ciphertext": self.ciphertext,
            "label": self.label,
        })
    }
}

/// スロットの組を確かめる。パスキーは credential ID ごとに 1 つ、リカバリーキーは 1 つまで。
fn validate_slot_set(slots: &[SlotInput]) -> Result<(), AppError> {
    if slots.is_empty() {
        return Err(slots_required());
    }
    let mut ids = BTreeSet::new();
    let mut credentials = BTreeSet::new();
    let mut recovery = 0;
    for slot in slots {
        slot.validate()?;
        if !ids.insert(slot.id.as_str()) {
            return Err(bad("slots", "duplicate slot id"));
        }
        match &slot.credential_id {
            Some(credential_id) if !credentials.insert(credential_id.as_str()) => {
                return Err(bad("slots", "duplicate credential_id"));
            }
            None => recovery += 1,
            _ => {}
        }
    }
    if recovery > 1 {
        return Err(bad("slots", "at most one recovery slot"));
    }
    if credentials.len() > MAX_PASSKEY_SLOTS {
        return Err(limit_exceeded("slots"));
    }
    Ok(())
}

fn slots_required() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_SLOTS_REQUIRED",
        "At least one key slot is required",
        Value::Null,
    )
}

pub(crate) fn ssss_conflict(current: Option<&Value>) -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_SSSS_CONFLICT",
        "The secret storage has changed",
        json!({
            "default_key_id": current.map_or(Value::Null, |c| c["key_id"].clone()),
            "secrets_revision": current.map_or(Value::Null, |c| c["secrets_revision"].clone()),
        }),
    )
}

// ---------------------------------------------------------------------------
// DB

const SLOT_JSON: &str = "json_build_object('id', s.id, 'key_id', s.key_id, 'kind', s.kind, 'credential_id', s.credential_id, 'prf_salt', s.prf_salt, 'nonce', s.nonce, 'ciphertext', s.ciphertext, 'label', s.label, 'created_at', s.created_at, 'last_used_at', s.last_used_at)";

/// 既定の SSSS 鍵（`{"key_id", "secrets_revision"}`）。
pub(crate) async fn default_key(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Option<Value>, AppError> {
    first(
        tx,
        "SELECT json_build_object('key_id', key_id, 'secrets_revision', secrets_revision)::text
         FROM e2ee_secret_storage_keys WHERE user_id = $1 AND is_default",
        &[user_id],
    )
    .await
}

/// クロス署名鍵と鍵バックアップの版の保存の前提にした既定の SSSS 鍵（`expected_default_key_id`）を、
/// ユーザー単位のロックのあとの同じトランザクションで照合する（#360）。違えば 409
/// `E2EE_SSSS_CONFLICT`。初回設定とリセットで、別の端末が同時に SSSS を作り直したときに、SSSS と
/// サーバーの公開鍵の世代がずれないようにするため。省略したら照合しない。
pub(crate) async fn check_expected_default_key(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    expected: Option<&str>,
) -> Result<(), AppError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let current = default_key(tx, user_id).await?;
    if current.as_ref().and_then(|c| c["key_id"].as_str()) != Some(expected) {
        return Err(ssss_conflict(current.as_ref()));
    }
    Ok(())
}

async fn insert_slots(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    key_id: &str,
    slots: &[SlotInput],
) -> Result<(), AppError> {
    let ids = json!(slots.iter().map(|s| &s.id).collect::<Vec<_>>()).to_string();
    // slot_id は表全体の主キー。ほかのユーザーや消えていない別のスロットの ID は使えない。
    let taken = first(
        tx,
        "SELECT '{}'::text FROM e2ee_secret_storage_key_slots
         WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb)) LIMIT 1",
        &[&ids],
    )
    .await?;
    if taken.is_some() {
        return Err(bad("slot.id", "already used"));
    }
    exec(
        tx,
        "INSERT INTO e2ee_secret_storage_key_slots
           (id, user_id, key_id, kind, credential_id, prf_salt, nonce, ciphertext, label)
         SELECT s.id, $1, $2, s.kind, s.credential_id, s.prf_salt, s.nonce, s.ciphertext, s.label
         FROM jsonb_to_recordset($3::jsonb) AS s(id text, kind text, credential_id text,
           prf_salt text, nonce text, ciphertext text, label text)",
        &[
            user_id,
            key_id,
            &Value::Array(slots.iter().map(SlotInput::json).collect()).to_string(),
        ],
    )
    .await
}

// ---------------------------------------------------------------------------
// ハンドラー

/// `GET /api/e2ee/secret_storage`：解錠に必要なもの（既定の鍵、秘密、スロット）をまとめて返す。
async fn get_secret_storage(
    State(state): State<AppState>,
    actor: AuthUser,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let storage = first(
        &mut tx,
        &format!(
            "SELECT json_build_object(
               'default_key_id', k.key_id,
               'keys', json_build_object(k.key_id,
                 json_build_object('algorithm', k.algorithm, 'iv', k.iv, 'mac', k.mac)),
               'secrets', COALESCE((SELECT json_object_agg(x.name, json_build_object(
                   'key_id', x.key_id, 'iv', x.iv, 'ciphertext', x.ciphertext, 'mac', x.mac))
                 FROM e2ee_secrets x WHERE x.user_id = k.user_id AND x.key_id = k.key_id), '{{}}'::json),
               'secrets_revision', k.secrets_revision,
               'slots', COALESCE((SELECT json_agg({SLOT_JSON} ORDER BY s.created_at, s.id)
                 FROM e2ee_secret_storage_key_slots s
                 WHERE s.user_id = k.user_id AND s.key_id = k.key_id), '[]'::json)
             )::text
             FROM e2ee_secret_storage_keys k WHERE k.user_id = $1 AND k.is_default"
        ),
        &[&actor.id],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(storage.unwrap_or_else(|| {
        json!({"default_key_id": null, "keys": {}, "secrets": {}, "secrets_revision": null, "slots": []})
    })))
}

#[derive(Deserialize)]
pub struct CreateKeyRequest {
    #[serde(default, deserialize_with = "present")]
    expected_default_key_id: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    expected_secrets_revision: Option<Value>,
    key_id: String,
    key: KeyDescription,
    secrets: std::collections::BTreeMap<String, EncryptedSecret>,
    #[serde(default)]
    slots: Vec<SlotInput>,
}

/// `POST /api/e2ee/secret_storage/keys`：新しい SSSS 鍵を作って既定にし、秘密とスロットを入れ替える。
async fn create_key(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreateKeyRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    // スロットのない SSSS は、どの手段でも開けない。
    validate_slot_set(&req.slots)?;
    check_uuid("key_id", &req.key_id)?;
    req.key.validate()?;
    validate_secrets(&req.secrets)?;
    if let Some(missing) = REQUIRED_SECRETS
        .iter()
        .find(|name| !req.secrets.contains_key(**name))
    {
        return Err(bad("secrets", &format!("{missing} is required")));
    }
    let reauth = reauth(&state, &headers, &actor).await?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    // 初回設定（既定の鍵がないとき）も含め、直前の再認証が要る。
    reauth.require()?;
    let current = default_key(&mut tx, &actor.id).await?;
    let current_key = current
        .as_ref()
        .map_or(Value::Null, |c| c["key_id"].clone());
    let current_revision = current
        .as_ref()
        .map_or(Value::Null, |c| c["secrets_revision"].clone());
    for (expected, now, name) in [
        (
            &req.expected_default_key_id,
            &current_key,
            "expected_default_key_id",
        ),
        (
            &req.expected_secrets_revision,
            &current_revision,
            "expected_secrets_revision",
        ),
    ] {
        match expected {
            None if current.is_some() => {
                return Err(AppError::BadRequest(format!(
                    "{name} is required unless no secret storage exists"
                )));
            }
            None => {}
            Some(expected) if expected != now => return Err(ssss_conflict(current.as_ref())),
            Some(_) => {}
        }
    }
    if current_key.as_str() == Some(req.key_id.as_str()) {
        return Err(bad("key_id", "must be a new key ID"));
    }
    let revision = current_revision.as_i64().unwrap_or(0) + 1;

    // 古い鍵を消すと、その秘密とスロットも消える（外部キーの ON DELETE CASCADE）。
    exec(
        &mut tx,
        "DELETE FROM e2ee_secret_storage_keys WHERE user_id = $1",
        &[&actor.id],
    )
    .await?;
    exec(
        &mut tx,
        "INSERT INTO e2ee_secret_storage_keys
           (user_id, key_id, algorithm, iv, mac, is_default, secrets_revision)
         VALUES ($1, $2, $3, $4, $5, true, $6::bigint)",
        &[
            &actor.id,
            &req.key_id,
            &req.key.algorithm,
            &req.key.iv,
            &req.key.mac,
            &revision.to_string(),
        ],
    )
    .await?;
    exec(
        &mut tx,
        "INSERT INTO e2ee_secrets (user_id, name, key_id, iv, ciphertext, mac)
         SELECT $1, s.name, $2, s.iv, s.ciphertext, s.mac
         FROM jsonb_to_recordset($3::jsonb) AS s(name text, iv text, ciphertext text, mac text)",
        &[&actor.id, &req.key_id, &secrets_json(&req.secrets)],
    )
    .await?;
    insert_slots(&mut tx, &actor.id, &req.key_id, &req.slots).await?;
    tx.commit().await?;
    // SSSS は Shape で配らないので、観測できる変更はない。
    Ok(Json(ApiResponse::new(
        json!({"default_key_id": req.key_id, "secrets_revision": revision}),
        None,
    )))
}

#[derive(Deserialize)]
pub struct PutSecretsRequest {
    key_id: String,
    #[serde(default, deserialize_with = "present")]
    expected_secrets_revision: Option<Value>,
    secrets: std::collections::BTreeMap<String, EncryptedSecret>,
}

/// `POST /api/e2ee/secret_storage/secrets`：既定の鍵で暗号化した秘密を足すか置き換える。
async fn put_secrets(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<PutSecretsRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    if req.secrets.is_empty() {
        return Err(bad("secrets", "must not be empty"));
    }
    validate_secrets(&req.secrets)?;
    let reauth = reauth(&state, &headers, &actor).await?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    let current = default_key(&mut tx, &actor.id).await?;
    let Some(current) = current.filter(|c| c["key_id"].as_str() == Some(req.key_id.as_str()))
    else {
        let current = default_key(&mut tx, &actor.id).await?;
        return Err(ssss_conflict(current.as_ref()));
    };
    match &req.expected_secrets_revision {
        None => {
            return Err(AppError::BadRequest(
                "expected_secrets_revision is required".into(),
            ));
        }
        Some(expected) if *expected != current["secrets_revision"] => {
            return Err(ssss_conflict(Some(&current)));
        }
        Some(_) => {}
    }
    let names = json!(req.secrets.keys().collect::<Vec<_>>()).to_string();
    let replaces = first(
        &mut tx,
        "SELECT '{}'::text FROM e2ee_secrets
         WHERE user_id = $1 AND name IN (SELECT jsonb_array_elements_text($2::jsonb)) LIMIT 1",
        &[&actor.id, &names],
    )
    .await?
    .is_some();
    if replaces {
        // 前に取得した古い暗号文を送り直して、失効の前の秘密鍵に戻せないようにする。
        reauth.require()?;
    }
    exec(
        &mut tx,
        "INSERT INTO e2ee_secrets (user_id, name, key_id, iv, ciphertext, mac)
         SELECT $1, s.name, $2, s.iv, s.ciphertext, s.mac
         FROM jsonb_to_recordset($3::jsonb) AS s(name text, iv text, ciphertext text, mac text)
         ON CONFLICT (user_id, name) DO UPDATE
         SET key_id = EXCLUDED.key_id, iv = EXCLUDED.iv, ciphertext = EXCLUDED.ciphertext,
             mac = EXCLUDED.mac, updated_at = now()",
        &[&actor.id, &req.key_id, &secrets_json(&req.secrets)],
    )
    .await?;
    let revision = first(
        &mut tx,
        "UPDATE e2ee_secret_storage_keys SET secrets_revision = secrets_revision + 1
         WHERE user_id = $1 AND key_id = $2 RETURNING secrets_revision::text",
        &[&actor.id, &req.key_id],
    )
    .await?
    .unwrap_or(Value::Null);
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"secrets_revision": revision}),
        None,
    )))
}

#[derive(Deserialize)]
pub struct AddSlotRequest {
    key_id: String,
    #[serde(flatten)]
    slot: SlotInput,
}

/// `POST /api/e2ee/secret_storage/slots`：既定の鍵にスロットを足す。同じ credential ID のパスキーの
/// スロットか、リカバリーキーのスロットがあれば置き換える（置き換えには直前の再認証が要る）。
async fn add_slot(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<AddSlotRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    req.slot.validate()?;
    let reauth = reauth(&state, &headers, &actor).await?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    let current = default_key(&mut tx, &actor.id).await?;
    if current.as_ref().and_then(|c| c["key_id"].as_str()) != Some(req.key_id.as_str()) {
        // すでに消えた鍵のスロットを足して、「最後の 1 つ」の規則を満たさないようにする。
        return Err(ssss_conflict(current.as_ref()));
    }
    let slot_sql = format!(
        "SELECT {SLOT_JSON}::text FROM e2ee_secret_storage_key_slots s WHERE s.id = $1 AND s.user_id = $2"
    );
    // 応答をなくした同じスロットの再送は、何もせずに 200。
    if let Some(existing) = first(&mut tx, &slot_sql, &[&req.slot.id, &actor.id]).await? {
        let same = existing["key_id"] == req.key_id
            && [
                "kind",
                "credential_id",
                "prf_salt",
                "nonce",
                "ciphertext",
                "label",
            ]
            .iter()
            .all(|f| existing[*f] == req.slot.json()[*f]);
        if same {
            tx.commit().await?;
            return Ok(Json(ApiResponse::new(existing, None)));
        }
        return Err(bad("slot.id", "already used"));
    }
    let replaced = first(
        &mut tx,
        "SELECT to_json(id)::text FROM e2ee_secret_storage_key_slots
         WHERE user_id = $1 AND key_id = $2 AND kind = $3
           AND ($3 = 'recovery' OR credential_id = $4)",
        &[
            &actor.id,
            &req.key_id,
            &req.slot.kind,
            req.slot.credential_id.as_deref().unwrap_or(""),
        ],
    )
    .await?;
    if let Some(replaced) = replaced.as_ref().and_then(Value::as_str) {
        reauth.require()?;
        exec(
            &mut tx,
            "DELETE FROM e2ee_secret_storage_key_slots WHERE id = $1 AND user_id = $2",
            &[replaced, &actor.id],
        )
        .await?;
    } else if req.slot.kind == "passkey" {
        let count = first(
            &mut tx,
            "SELECT count(*)::text FROM e2ee_secret_storage_key_slots
             WHERE user_id = $1 AND key_id = $2 AND kind = 'passkey'",
            &[&actor.id, &req.key_id],
        )
        .await?
        .and_then(|c| c.as_u64())
        .unwrap_or(0);
        if count as usize >= MAX_PASSKEY_SLOTS {
            return Err(limit_exceeded("slots"));
        }
    }
    insert_slots(
        &mut tx,
        &actor.id,
        &req.key_id,
        std::slice::from_ref(&req.slot),
    )
    .await?;
    let slot = first(&mut tx, &slot_sql, &[&req.slot.id, &actor.id])
        .await?
        .unwrap_or(Value::Null);
    tx.commit().await?;
    Ok(Json(ApiResponse::new(slot, None)))
}

/// `DELETE /api/e2ee/secret_storage/slots/{slot_id}`：スロットを消す。最後の 1 つは消せない。
async fn delete_slot(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Path(slot_id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let reauth = reauth(&state, &headers, &actor).await?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    // 件数の確認と削除を直列にする。取らないと、同時の 2 つの DELETE がどちらも「まだ 2 つある」と読んで、
    // 0 個にできてしまう。
    lock_user(&mut tx, &actor.id).await?;
    // 判定の順は、404（すでに消えたスロットの再試行と見分ける）、最後の 1 つの 400、再認証の 403。
    let slot = first(
        &mut tx,
        "SELECT json_build_object('key_id', key_id, 'count', (SELECT count(*) FROM e2ee_secret_storage_key_slots o
           WHERE o.user_id = s.user_id AND o.key_id = s.key_id))::text
         FROM e2ee_secret_storage_key_slots s WHERE s.id = $1 AND s.user_id = $2",
        &[&slot_id, &actor.id],
    )
    .await?
    .ok_or_else(|| AppError::NotFound("Slot not found".into()))?;
    if slot["count"].as_i64().unwrap_or(0) <= 1 {
        return Err(slots_required());
    }
    reauth.require()?;
    exec(
        &mut tx,
        "DELETE FROM e2ee_secret_storage_key_slots WHERE id = $1 AND user_id = $2",
        &[&slot_id, &actor.id],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(json!({"id": slot_id}), None)))
}

/// `POST /api/e2ee/secret_storage/slots/{slot_id}/used`：スロットを最後に使った日時を記録する。
async fn mark_slot_used(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(slot_id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let slot = first(
        &mut tx,
        "UPDATE e2ee_secret_storage_key_slots SET last_used_at = now()
         WHERE id = $1 AND user_id = $2 RETURNING json_build_object('id', id, 'last_used_at', last_used_at)::text",
        &[&slot_id, &actor.id],
    )
    .await?
    .ok_or_else(|| AppError::NotFound("Slot not found".into()))?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(slot, None)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(len: usize) -> String {
        BASE64.encode(vec![7u8; len])
    }

    fn slot(kind: &str, credential: Option<&str>) -> SlotInput {
        SlotInput {
            id: Uuid::new_v4().to_string(),
            kind: kind.into(),
            credential_id: credential.map(str::to_owned),
            prf_salt: credential.map(|_| b64(32)),
            nonce: b64(12),
            ciphertext: b64(48),
            label: String::new(),
        }
    }

    #[test]
    fn validates_slot_shapes() {
        slot("passkey", Some("cred-1_A")).validate().unwrap();
        slot("recovery", None).validate().unwrap();
        assert!(slot("passkey", None).validate().is_err());
        assert!(slot("passkey", Some("not+base64url")).validate().is_err());
        let mut bad_nonce = slot("recovery", None);
        bad_nonce.nonce = b64(16);
        assert!(bad_nonce.validate().is_err());
        let mut upper = slot("recovery", None);
        upper.id = upper.id.to_uppercase();
        assert!(upper.validate().is_err());
        // パディング付きの base64 も受け付ける。
        let mut padded = slot("recovery", None);
        padded.nonce = base64::engine::general_purpose::STANDARD.encode([1u8; 12]);
        padded.validate().unwrap();
    }

    #[test]
    fn slot_sets_need_one_slot_and_cap_passkeys() {
        assert!(matches!(
            validate_slot_set(&[]),
            Err(AppError::Coded {
                code: "E2EE_SLOTS_REQUIRED",
                ..
            })
        ));
        let passkeys: Vec<SlotInput> = (0..=MAX_PASSKEY_SLOTS)
            .map(|i| slot("passkey", Some(&format!("cred{i}"))))
            .collect();
        assert!(matches!(
            validate_slot_set(&passkeys),
            Err(AppError::Coded {
                code: "E2EE_LIMIT_EXCEEDED",
                ..
            })
        ));
        validate_slot_set(&passkeys[..MAX_PASSKEY_SLOTS]).unwrap();
        assert!(validate_slot_set(&[slot("recovery", None), slot("recovery", None)]).is_err());
        let same = slot("passkey", Some("same"));
        let mut again = slot("passkey", Some("same"));
        again.id = Uuid::new_v4().to_string();
        assert!(validate_slot_set(&[same, again]).is_err());
    }
}
