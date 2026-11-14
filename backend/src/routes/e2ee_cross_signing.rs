//! E2EE のクロス署名鍵、署名、TOFU のピンの API（#356）。仕様は `docs/e2ee.md` の「クロス署名」。
//!
//! 書き込みのロックの順は `routes/e2ee.rs` の冒頭と同じ（`users` の行を `FOR SHARE`、ユーザー単位の
//! advisory lock の順）。判定と書き込みは、ロックを取ったあとの同じトランザクションで行う。

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};
use zoo_crypto::Ed25519PublicKey;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    routes::{
        e2ee::{
            MAX_SIGNATURES_PER_UPLOAD, SHARED_USERS_SQL, bad_signature, check_key_size, coded,
            crypto_error, exec, first, limit_exceeded, lock_active_user, lock_device, lock_user,
            rows, signature_count,
        },
        e2ee_reauth::reauth,
        e2ee_secret_storage::check_expected_default_key,
        response::ApiResponse,
    },
    state::AppState,
};

/// クロス署名鍵の用途。
pub const USAGES: [&str; 3] = ["master", "self_signing", "user_signing"];
/// `POST /api/e2ee/tofu_pins` で 1 回に送れるピンの数。
pub const MAX_PINS_PER_REQUEST: usize = 1000;
/// `GET /api/e2ee/tofu_pins` の `limit` の上限と既定値。
pub const MAX_PINS_PAGE: i64 = 1000;
/// TOFU のピンの JSON の型。
pub const TOFU_PIN_TYPE: &str = "zoo.tofu_pin";

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/e2ee/cross_signing/keys",
            post(upload_cross_signing_keys),
        )
        .route(
            "/api/e2ee/cross_signing/signatures",
            post(upload_signatures),
        )
        .route("/api/e2ee/tofu_pins", get(list_pins).post(upload_pins))
}

/// 省略（`None`）と `null`（`Some(Value::Null)`）を区別して読む。`#[serde(default)]` と一緒に使う。
pub(crate) fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

/// パディングなし base64 の Ed25519 公開鍵を読む。表記の揺れ（パディング付きなど）で同じ鍵を別の文字列として
/// 登録し直せないよう、正準な表記（`to_base64()` と同じ）だけを受け付ける。
pub(crate) fn parse_ed25519(key: &str) -> Result<Ed25519PublicKey, AppError> {
    let parsed = Ed25519PublicKey::from_base64(key).map_err(|e| crypto_error(e.into()))?;
    if parsed.to_base64() != key {
        return Err(AppError::BadRequest(
            "Keys must be unpadded standard base64".into(),
        ));
    }
    Ok(parsed)
}

/// `value` の `signatures.<user_id>.ed25519:<signer>` が `signer` の署名として正しいか。
pub(crate) fn verify_signed_by(value: &Value, user_id: &str, signer: &str) -> Result<(), AppError> {
    let key = parse_ed25519(signer)?;
    zoo_crypto::verify_json_signature(value, user_id, &format!("ed25519:{signer}"), &key)
        .map_err(crypto_error)
}

/// 検証したクロス署名鍵。
#[derive(Debug, Clone, PartialEq)]
struct CrossSigningKey {
    public_key: String,
    json: Value,
}

/// クロス署名鍵の JSON（`{"user_id", "usage": [usage], "keys": {"ed25519:<公開鍵>": "<公開鍵>"}, "signatures"}`）を読む。
fn parse_cross_signing_key(
    value: &Value,
    user_id: &str,
    usage: &str,
) -> Result<CrossSigningKey, AppError> {
    check_key_size(value)?;
    let invalid = |why: &str| AppError::BadRequest(format!("Invalid {usage} key: {why}"));
    if value.get("user_id").and_then(Value::as_str) != Some(user_id) {
        return Err(invalid("user_id must be the caller"));
    }
    if value.get("usage") != Some(&json!([usage])) {
        return Err(invalid("usage does not match"));
    }
    let keys = value
        .get("keys")
        .and_then(Value::as_object)
        .filter(|keys| keys.len() == 1)
        .ok_or_else(|| invalid("keys must have exactly one key"))?;
    let (name, key) = keys.iter().next().expect("one key");
    let key = key
        .as_str()
        .ok_or_else(|| invalid("key must be a string"))?;
    if name.strip_prefix("ed25519:") != Some(key) {
        return Err(invalid("key ID must be ed25519:<public key>"));
    }
    parse_ed25519(key)?;
    if value.get("signatures").is_some_and(|s| !s.is_object()) {
        return Err(invalid("signatures must be an object"));
    }
    Ok(CrossSigningKey {
        public_key: key.to_owned(),
        json: value.clone(),
    })
}

/// 今のクロス署名鍵（用途ごと）。
async fn current_keys(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<BTreeMap<String, CrossSigningKey>, AppError> {
    Ok(rows(
        tx,
        "SELECT json_build_object('usage', usage, 'public_key', public_key, 'json', key_json)::text
         FROM e2ee_cross_signing_keys WHERE user_id = $1",
        &[user_id],
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        Some((
            row["usage"].as_str()?.to_owned(),
            CrossSigningKey {
                public_key: row["public_key"].as_str()?.to_owned(),
                json: row["json"].clone(),
            },
        ))
    })
    .collect())
}

fn keys_response(keys: &BTreeMap<String, CrossSigningKey>) -> Value {
    let mut map = Map::new();
    for usage in USAGES {
        let name = format!("{usage}_key");
        map.insert(
            name,
            keys.get(usage).map_or(Value::Null, |k| k.json.clone()),
        );
    }
    Value::Object(map)
}

fn public_keys(keys: &BTreeMap<String, CrossSigningKey>) -> Value {
    let mut map = Map::new();
    for usage in USAGES {
        map.insert(
            usage.into(),
            keys.get(usage)
                .map_or(Value::Null, |k| Value::from(k.public_key.clone())),
        );
    }
    Value::Object(map)
}

fn key_reused() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_KEY_REUSED",
        "The key was used before",
        Value::Null,
    )
}

#[derive(Deserialize)]
pub struct CrossSigningRequest {
    #[serde(default)]
    master_key: Option<Value>,
    #[serde(default)]
    self_signing_key: Option<Value>,
    #[serde(default)]
    user_signing_key: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    expected_keys: Option<Value>,
    /// 前提にした既定の SSSS 鍵（初回設定、リセット、失効。#360、#362）。省略したら照合しない。
    #[serde(default)]
    expected_default_key_id: Option<String>,
    /// 前提にした `secrets_revision`（失効。#362）。端末だけの失効は SSSS の鍵を変えないので、別の端末が
    /// 秘密だけを書き換えたことは、これで見分ける。省略したら照合しない。
    #[serde(default)]
    expected_secrets_revision: Option<i64>,
}

/// `POST /api/e2ee/cross_signing/keys`：クロス署名鍵を登録するか差し替える。
async fn upload_cross_signing_keys(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CrossSigningRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let mut provided = BTreeMap::new();
    for (usage, value) in [
        ("master", &req.master_key),
        ("self_signing", &req.self_signing_key),
        ("user_signing", &req.user_signing_key),
    ] {
        if let Some(value) = value.as_ref().filter(|v| !v.is_null()) {
            provided.insert(
                usage.to_owned(),
                parse_cross_signing_key(value, &actor.id, usage)?,
            );
        }
    }
    if provided.is_empty() {
        return Err(AppError::BadRequest("No key to upload".into()));
    }
    if provided
        .values()
        .map(|k| signature_count(&k.json))
        .sum::<usize>()
        > MAX_SIGNATURES_PER_UPLOAD
    {
        return Err(limit_exceeded("signatures"));
    }
    let reauth = reauth(&state, &headers, &actor).await?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    check_expected_default_key(
        &mut tx,
        &actor.id,
        req.expected_default_key_id.as_deref(),
        req.expected_secrets_revision,
    )
    .await?;
    let current = current_keys(&mut tx, &actor.id).await?;

    // 公開鍵と key_json（署名を含む）がどちらも今と同じなら、何も変えずに 200（応答をなくした再送）。
    // expected_keys の照合と再認証より先に判定する。
    if provided
        .iter()
        .all(|(usage, key)| current.get(usage) == Some(key))
    {
        tx.commit().await?;
        return Ok(Json(ApiResponse::new(keys_response(&current), None)));
    }

    // 初めての登録（初回設定）を含め、変更には直前の再認証が要る。
    reauth.require()?;
    match &req.expected_keys {
        None if !current.is_empty() => {
            return Err(AppError::BadRequest(
                "expected_keys is required unless no key is registered".into(),
            ));
        }
        None => {}
        Some(expected) => {
            let current_public = public_keys(&current);
            // 省いた用途は null（鍵がない）として扱う。
            let matches = expected.is_object()
                && USAGES.iter().all(|usage| {
                    expected.get(*usage).unwrap_or(&Value::Null) == &current_public[*usage]
                });
            if !matches {
                return Err(coded(
                    StatusCode::CONFLICT,
                    "E2EE_CROSS_SIGNING_CONFLICT",
                    "The cross-signing keys have changed",
                    json!({"current_keys": public_keys(&current)}),
                ));
            }
        }
    }

    let master = provided
        .get("master")
        .or_else(|| current.get("master"))
        .ok_or_else(|| AppError::BadRequest("A master key is required".into()))?
        .clone();
    let master_replaced = current
        .get("master")
        .is_some_and(|m| m.public_key != master.public_key);
    if master_replaced
        && !(provided.contains_key("self_signing") && provided.contains_key("user_signing"))
    {
        // 古いマスター鍵の署名のまま残ると、クライアントが新しいマスター鍵で検証できない。
        return Err(AppError::BadRequest(
            "Replacing the master key requires new self-signing and user-signing keys".into(),
        ));
    }
    for usage in ["self_signing", "user_signing"] {
        if let Some(key) = provided.get(usage) {
            verify_signed_by(&key.json, &actor.id, &master.public_key)?;
        }
    }

    // 用途をまたいでも同じ公開鍵は使わない。
    let mut finals = BTreeSet::new();
    for usage in USAGES {
        if let Some(key) = provided.get(usage).or_else(|| current.get(usage))
            && !finals.insert(key.public_key.clone())
        {
            return Err(AppError::BadRequest(
                "Cross-signing keys must be distinct".into(),
            ));
        }
    }

    // 登録し直しの拒否：過去に使った鍵（変更の記録の古い公開鍵）と、今ほかの用途で使っている鍵。
    let changed: Vec<(&String, &CrossSigningKey)> = provided
        .iter()
        .filter(|(usage, key)| current.get(*usage) != Some(*key))
        .collect();
    let new_public_keys: Vec<&str> = changed
        .iter()
        .filter(|(usage, key)| current.get(*usage).map(|c| &c.public_key) != Some(&key.public_key))
        .map(|(_, key)| key.public_key.as_str())
        .collect();
    if !new_public_keys.is_empty() {
        if new_public_keys
            .iter()
            .any(|pk| current.values().any(|c| c.public_key == *pk))
        {
            return Err(key_reused());
        }
        let reused = first(
            &mut tx,
            "SELECT '{}'::text FROM e2ee_cross_signing_key_changes
             WHERE user_id = $1 AND old_public_key IN (SELECT jsonb_array_elements_text($2::jsonb))
             LIMIT 1",
            &[&actor.id, &json!(new_public_keys).to_string()],
        )
        .await?;
        if reused.is_some() {
            return Err(key_reused());
        }
    }

    // 公開鍵が変わる（初回の登録を含む）なら、参加している暗号化チャンネルの membership_epoch を増やす。
    // 漏れた鍵で署名された端末に、今のセッションの続きが届かないようにするため。鍵の行を書く前に、
    // ロックの順（ユーザー単位のロック、channels の行）のとおりに取る。
    if !new_public_keys.is_empty() {
        crate::routes::e2ee_channels::bump_user_epochs(&mut tx, &actor.id).await?;
    }

    for (usage, key) in &changed {
        let old = current.get(*usage);
        exec(
            &mut tx,
            "INSERT INTO e2ee_cross_signing_keys (user_id, usage, public_key, key_json)
             VALUES ($1, $2, $3, $4::jsonb)
             ON CONFLICT (user_id, usage) DO UPDATE
             SET public_key = EXCLUDED.public_key, key_json = EXCLUDED.key_json, updated_at = now()",
            &[&actor.id, usage, &key.public_key, &key.json.to_string()],
        )
        .await?;
        if old.map(|o| &o.public_key) == Some(&key.public_key) {
            // 同じ公開鍵の署名だけが変わった（新しいマスター鍵での再署名など）。鍵の変更ではない。
            continue;
        }
        let old_public_key = old.map(|o| o.public_key.clone());
        let mut statement = toasty::sql::statement(
            "INSERT INTO e2ee_cross_signing_key_changes (user_id, usage, old_public_key, new_public_key)
             VALUES ($1, $2, NULLIF($3, ''), $4)",
        );
        statement = statement
            .bind(actor.id.as_str())
            .bind(usage.as_str())
            .bind(old_public_key.as_deref().unwrap_or(""))
            .bind(key.public_key.as_str());
        statement.exec(&mut tx).await?;
        if let Some(old) = old_public_key {
            // 古い鍵による署名と、古いマスター鍵への署名を消す。
            exec(
                &mut tx,
                "DELETE FROM e2ee_key_signatures
                 WHERE (signer_user_id = $1 AND signer_key_id = 'ed25519:' || $2)
                    OR ($3 = 'master' AND target_user_id = $1 AND target_key_id = $2)",
                &[&actor.id, &old, usage],
            )
            .await?;
        }
    }

    let keys = current_keys(&mut tx, &actor.id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(keys_response(&keys), txid)))
}

// ---------------------------------------------------------------------------
// 署名

#[derive(Deserialize)]
pub struct SignatureItem {
    target_user_id: String,
    target_key_id: String,
    signer_key_id: String,
    signature: String,
}

#[derive(Deserialize)]
pub struct SignaturesRequest {
    signatures: Vec<SignatureItem>,
}

/// `POST /api/e2ee/cross_signing/signatures`：自己署名鍵による自分の端末への署名と、ユーザー署名鍵による
/// 他のユーザーのマスター鍵への署名を足す。署名者は常に呼び出したユーザー。
async fn upload_signatures(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<SignaturesRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    if req.signatures.is_empty() {
        return Err(AppError::BadRequest("No signature to upload".into()));
    }
    if req.signatures.len() > MAX_SIGNATURES_PER_UPLOAD {
        return Err(limit_exceeded("signatures"));
    }
    let mut seen = BTreeSet::new();
    for item in &req.signatures {
        if item.signature.len() > 256 {
            return Err(bad_signature());
        }
        if !seen.insert((
            &item.target_user_id,
            &item.target_key_id,
            &item.signer_key_id,
        )) {
            return Err(AppError::BadRequest("Duplicate signature".into()));
        }
    }
    let reauth = reauth(&state, &headers, &actor).await?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 無効化されたユーザーの書き込みは 403（端末の登録と同じ）。
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    let current = current_keys(&mut tx, &actor.id).await?;
    let signer_of = |usage: &str| {
        current
            .get(usage)
            .map(|k| format!("ed25519:{}", k.public_key))
    };
    let (self_signing, user_signing) = (signer_of("self_signing"), signer_of("user_signing"));

    // 自己署名の対象の端末は、端末の削除と同じ端末単位のロックを取ってから読む。
    // 取らないと、端末を読んだあとに削除（署名の削除を含む）がコミットし、その後の
    // INSERT で削除済みの端末に署名が残る。順序は全体の規則どおり、ユーザー単位の
    // ロックのあとに device_id の昇順（BTreeSet）で取る。
    let self_signed_devices: BTreeSet<&str> = req
        .signatures
        .iter()
        .filter(|s| Some(&s.signer_key_id) == self_signing.as_ref() && s.target_user_id == actor.id)
        .map(|s| s.target_key_id.as_str())
        .collect();
    for device_id in self_signed_devices {
        lock_device(&mut tx, &actor.id, device_id).await?;
    }

    let mut needs_reauth = false;
    for item in &req.signatures {
        let target = if Some(&item.signer_key_id) == self_signing.as_ref()
            && item.target_user_id == actor.id
        {
            // 自分の端末への自己署名（端末単位のロックは上で取ってある）。
            first(
                &mut tx,
                "SELECT device_keys::text FROM e2ee_devices
                 WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL",
                &[&actor.id, &item.target_key_id],
            )
            .await?
            .ok_or_else(|| AppError::NotFound("Device not found".into()))?
        } else if Some(&item.signer_key_id) == user_signing.as_ref()
            && item.target_user_id != actor.id
        {
            // 他のユーザーのマスター鍵への署名（検証済みの印）。直前の再認証が要る。
            // 対象の鍵の行を FOR SHARE で読み、相手のマスター鍵の差し替えと直列にする
            // （読んだあとに差し替えと古い鍵への署名の削除がコミットし、古い鍵への署名が
            // 残るのを防ぐ）。差し替えが先なら、コミットを待ってから読み直し、404 にする。
            needs_reauth = true;
            first(
                &mut tx,
                &format!(
                    "SELECT k.key_json::text FROM e2ee_cross_signing_keys k
                     WHERE k.user_id = $2 AND k.usage = 'master' AND k.public_key = $3
                       AND (k.user_id IN ({SHARED_USERS_SQL})
                         OR EXISTS (SELECT 1 FROM e2ee_tofu_pins p WHERE p.user_id = $1 AND p.target_user_id = $2)
                         OR EXISTS (SELECT 1 FROM e2ee_key_signatures s WHERE s.signer_user_id = $1 AND s.target_user_id = $2))
                     FOR SHARE OF k"
                ),
                &[&actor.id, &item.target_user_id, &item.target_key_id],
            )
            .await?
            .ok_or_else(|| AppError::NotFound("Master key not found".into()))?
        } else {
            // 今の自己署名鍵とユーザー署名鍵のどちらでもない。
            return Err(bad_signature());
        };
        let mut signed = target;
        let Value::Object(map) = &mut signed else {
            return Err(bad_signature());
        };
        let mut by_key = Map::new();
        by_key.insert(item.signer_key_id.clone(), item.signature.clone().into());
        let mut by_user = Map::new();
        by_user.insert(actor.id.clone(), Value::Object(by_key));
        map.insert("signatures".into(), Value::Object(by_user));
        let signer = item.signer_key_id.trim_start_matches("ed25519:");
        verify_signed_by(&signed, &actor.id, signer)?;
    }
    if needs_reauth {
        reauth.require()?;
    }

    let items = Value::Array(
        req.signatures
            .iter()
            .map(|s| {
                json!({"target_user_id": s.target_user_id, "target_key_id": s.target_key_id,
                       "signer_key_id": s.signer_key_id, "signature": s.signature})
            })
            .collect(),
    )
    .to_string();
    // 同じ署名の再送は何もしない（200）。違う署名なら置き換える。
    let written = first(
        &mut tx,
        "WITH written AS (
         INSERT INTO e2ee_key_signatures (signer_user_id, signer_key_id, target_user_id, target_key_id, signature)
         SELECT $1, s.signer_key_id, s.target_user_id, s.target_key_id, s.signature
         FROM jsonb_to_recordset($2::jsonb)
           AS s(signer_key_id text, target_user_id text, target_key_id text, signature text)
         ON CONFLICT (signer_user_id, signer_key_id, target_user_id, target_key_id) DO UPDATE
         SET signature = EXCLUDED.signature, created_at = now()
         WHERE e2ee_key_signatures.signature <> EXCLUDED.signature
         RETURNING 1)
         SELECT count(*)::text FROM written",
        &[&actor.id, &items],
    )
    .await?
    .and_then(|c| c.as_u64())
    .unwrap_or(0);
    // 行を書いたときだけ txid を返す。すべて同じ署名の再送なら、Shape に流れる変更がないので null。
    let txid = if written > 0 {
        Some(get_current_txid(&mut tx).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"count": req.signatures.len()}),
        txid,
    )))
}

// ---------------------------------------------------------------------------
// TOFU のピン

#[derive(Deserialize)]
pub struct PinItem {
    target_user_id: String,
    master_key: String,
    #[serde(default)]
    signatures: Value,
}

#[derive(Deserialize)]
pub struct PinsRequest {
    pins: Vec<PinItem>,
    #[serde(default)]
    replace: bool,
}

fn pin_conflict(pin: Value) -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_PIN_CONFLICT",
        "A pin for a different master key already exists",
        json!({ "pin": pin }),
    )
}

/// `POST /api/e2ee/tofu_pins`：TOFU のピンを足す（`replace: true` で置き換える）。
async fn upload_pins(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<PinsRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    if req.pins.is_empty() {
        return Err(AppError::BadRequest("No pin to upload".into()));
    }
    if req.pins.len() > MAX_PINS_PER_REQUEST {
        return Err(limit_exceeded("pins"));
    }
    let mut pins = Vec::with_capacity(req.pins.len());
    let mut targets = BTreeSet::new();
    for pin in &req.pins {
        if pin.target_user_id == actor.id {
            return Err(AppError::BadRequest("Cannot pin yourself".into()));
        }
        if !targets.insert(pin.target_user_id.as_str()) {
            return Err(AppError::BadRequest("Duplicate target_user_id".into()));
        }
        parse_ed25519(&pin.master_key)?;
        let json = json!({
            "type": TOFU_PIN_TYPE,
            "user_id": actor.id,
            "target_user_id": pin.target_user_id,
            "master_key": pin.master_key,
            "signatures": pin.signatures,
        });
        check_key_size(&json)?;
        pins.push(json);
    }
    let reauth = if req.replace {
        Some(reauth(&state, &headers, &actor).await?)
    } else {
        None
    };

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    if let Some(reauth) = reauth {
        // replace: true は、変わったマスター鍵の受け入れとユーザー署名鍵の作り直しだけで使う。
        reauth.require()?;
    }
    let user_signing = current_keys(&mut tx, &actor.id)
        .await?
        .remove("user_signing")
        .ok_or_else(|| AppError::BadRequest("No user-signing key is registered".into()))?
        .public_key;
    for pin in &pins {
        verify_signed_by(pin, &actor.id, &user_signing)?;
    }
    let target_list = json!(targets.iter().collect::<Vec<_>>()).to_string();
    let existing_users = first(
        &mut tx,
        "SELECT count(*)::text FROM users WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb))",
        &[&target_list],
    )
    .await?
    .and_then(|c| c.as_u64())
    .unwrap_or(0);
    if existing_users != targets.len() as u64 {
        return Err(AppError::NotFound("User not found".into()));
    }
    let existing: BTreeMap<String, Value> = rows(
        &mut tx,
        "SELECT json_build_object('target_user_id', target_user_id, 'pin', pin_json)::text
         FROM e2ee_tofu_pins
         WHERE user_id = $1 AND target_user_id IN (SELECT jsonb_array_elements_text($2::jsonb))",
        &[&actor.id, &target_list],
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        Some((
            row["target_user_id"].as_str()?.to_owned(),
            row["pin"].clone(),
        ))
    })
    .collect();

    let mut writes = Vec::new();
    for pin in pins {
        let target = pin["target_user_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        match existing.get(&target) {
            None => writes.push(pin),
            Some(old) if req.replace => {
                if *old != pin {
                    writes.push(pin);
                }
            }
            Some(old) => {
                // 今のユーザー署名鍵で検証できるピンだけを守る。検証できない（作り直す前の鍵の）ピンは置き換える。
                if verify_signed_by(old, &actor.id, &user_signing).is_ok() {
                    if old["master_key"] != pin["master_key"] {
                        return Err(pin_conflict(old.clone()));
                    }
                } else {
                    writes.push(pin);
                }
            }
        }
    }
    if !writes.is_empty() {
        exec(
            &mut tx,
            "INSERT INTO e2ee_tofu_pins (user_id, target_user_id, master_key, pin_json)
             SELECT $1, p ->> 'target_user_id', p ->> 'master_key', p
             FROM jsonb_array_elements($2::jsonb) AS p
             ON CONFLICT (user_id, target_user_id) DO UPDATE
             SET master_key = EXCLUDED.master_key, pin_json = EXCLUDED.pin_json, updated_at = now()",
            &[&actor.id, &Value::Array(writes).to_string()],
        )
        .await?;
    }
    let stored = rows(
        &mut tx,
        "SELECT pin_json::text FROM e2ee_tofu_pins
         WHERE user_id = $1 AND target_user_id IN (SELECT jsonb_array_elements_text($2::jsonb))
         ORDER BY target_user_id COLLATE \"C\"",
        &[&actor.id, &target_list],
    )
    .await?;
    tx.commit().await?;
    // ピンは Shape で配らないので、観測できる変更はない。
    Ok(Json(ApiResponse::new(json!({"pins": stored}), None)))
}

#[derive(Deserialize)]
pub struct PinsQuery {
    after: Option<String>,
    limit: Option<i64>,
}

/// `GET /api/e2ee/tofu_pins`：自分のピンをすべて、`target_user_id` の昇順に返す。
async fn list_pins(
    State(state): State<AppState>,
    actor: AuthUser,
    Query(query): Query<PinsQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(MAX_PINS_PAGE);
    if !(1..=MAX_PINS_PAGE).contains(&limit) {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {MAX_PINS_PAGE}"
        )));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let mut pins = rows(
        &mut tx,
        "SELECT json_build_object('target_user_id', target_user_id, 'pin', pin_json)::text
         FROM e2ee_tofu_pins
         WHERE user_id = $1 AND ($2 = '' OR target_user_id COLLATE \"C\" > $2)
         ORDER BY target_user_id COLLATE \"C\" LIMIT $3::bigint",
        &[
            &actor.id,
            query.after.as_deref().unwrap_or(""),
            &(limit + 1).to_string(),
        ],
    )
    .await?;
    tx.commit().await?;
    let next = if pins.len() as i64 > limit {
        pins.truncate(limit as usize);
        pins.last().map(|p| p["target_user_id"].clone())
    } else {
        None
    };
    Ok(Json(json!({
        "pins": pins.into_iter().map(|p| p["pin"].clone()).collect::<Vec<_>>(),
        "next": next.unwrap_or(Value::Null),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_json(user: &str, usage: &str, key: &str) -> Value {
        json!({"user_id": user, "usage": [usage], "keys": {format!("ed25519:{key}"): key}})
    }

    #[test]
    fn parses_cross_signing_keys_strictly() {
        let user = "00000000-0000-4000-8000-000000000001";
        let account =
            zoo_crypto::DeviceAccount::new(user, &uuid::Uuid::new_v4().to_string()).unwrap();
        let key = account.identity_keys().ed25519.to_base64();
        #[allow(non_snake_case)]
        let KEY = key.as_str();
        let parsed =
            parse_cross_signing_key(&key_json(user, "master", KEY), user, "master").unwrap();
        assert_eq!(parsed.public_key, KEY);
        // 別のユーザー、別の用途、鍵 ID の不一致、複数の鍵、パディング付きの表記は拒否する。
        assert!(
            parse_cross_signing_key(&key_json("other", "master", KEY), user, "master").is_err()
        );
        assert!(
            parse_cross_signing_key(&key_json(user, "self_signing", KEY), user, "master").is_err()
        );
        let mut mismatched = key_json(user, "master", KEY);
        mismatched["keys"] = json!({"ed25519:other": KEY});
        assert!(parse_cross_signing_key(&mismatched, user, "master").is_err());
        let mut two = key_json(user, "master", KEY);
        two["keys"]["ed25519:x"] = json!("x");
        assert!(parse_cross_signing_key(&two, user, "master").is_err());
        let padded = format!("{KEY}=");
        assert!(
            parse_cross_signing_key(&key_json(user, "master", &padded), user, "master").is_err()
        );
    }

    #[test]
    fn present_distinguishes_null_from_missing() {
        #[derive(Deserialize)]
        struct T {
            #[serde(default, deserialize_with = "present")]
            v: Option<Value>,
        }
        assert_eq!(serde_json::from_str::<T>("{}").unwrap().v, None);
        assert_eq!(
            serde_json::from_str::<T>(r#"{"v": null}"#).unwrap().v,
            Some(Value::Null)
        );
    }
}
