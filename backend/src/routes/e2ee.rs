//! E2EE の端末と鍵の API（#354）。仕様は `docs/e2ee.md` の「API」「DB」「membership_epoch と送信の順序」。
//!
//! ロックの順は、すべての書き込みで次にそろえる（逆の順で取る経路を作らない）。
//!
//! 1. `users` の行（登録と鍵のアップロードは `FOR SHARE`。無効化の `UPDATE` は `FOR NO KEY UPDATE`）
//! 2. ユーザー単位の advisory lock `(1, hashtext(user_id))`
//! 3. 端末単位の advisory lock `(2, hashtext(user_id || ':' || device_id))`（複数なら昇順）
//! 4. `channels` の行（`id` の昇順。`membership_epoch` を増やす #357 で足す）
//! 5. `e2ee_devices` の行

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;
use zoo_crypto::{CryptoError, Curve25519PublicKey, Ed25519PublicKey, SIGNED_CURVE25519};

use crate::{
    auth::AuthUser, db::get_current_txid, error::AppError, routes::response::ApiResponse,
    state::AppState,
};

/// 1 ユーザーの端末（削除済みを除く）の上限。
pub const MAX_DEVICES_PER_USER: i64 = 20;
/// 端末の登録の、1 ユーザーあたり 1 時間の回数の上限（削除済みの端末の登録も数える）。
pub const MAX_REGISTRATIONS_PER_HOUR: i64 = 30;
/// 1 端末のワンタイム鍵とフォールバック鍵の合計の上限。
pub const MAX_KEYS_PER_DEVICE: i64 = 100;
/// 端末鍵と、ワンタイム鍵とフォールバック鍵の 1 つずつの JSON の大きさの上限（バイト）。
pub const MAX_KEY_JSON_BYTES: usize = 8 * 1024;
/// 1 回のアップロード（登録を含む）に含める署名の件数の上限。
pub const MAX_SIGNATURES_PER_UPLOAD: usize = 1000;
/// `keys/query` で 1 回に問い合わせるユーザーの上限。
pub const MAX_QUERY_USERS: usize = 100;
/// `keys/query` の応答の合計の大きさの上限（バイト）。
pub const MAX_QUERY_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// 呼び出したユーザーと宛先の端末の組ごとの、1 日（UTC）のワンタイム鍵の払い出しの上限。
pub const MAX_CLAIMS_PER_DAY: i64 = 20;
/// `keys/claim` で 1 回に払い出す端末の上限。
pub const MAX_CLAIM_DEVICES: usize = 1000;
/// 端末名の長さの上限（文字数）。
const MAX_DISPLAY_NAME_CHARS: usize = 255;

/// 自分と暗号化チャンネルを共有しているユーザー（`$1` は自分）。削除済みのチャンネルは含めない。
/// 端末の Shape、`keys/query`、`keys/claim` で同じ範囲を使う。
///
/// Electric の where は別名付きの列（`cu.user_id`）を受け付けないので、別名を使わない。
/// 列名はいちばん内側の表から解決されるので、外側の表（`e2ee_devices` など）の列と取り違えない。
pub(crate) const SHARED_USERS_SQL: &str = "SELECT user_id FROM channel_user WHERE channel_id IN (SELECT id FROM channels WHERE encrypted AND deleted_at IS NULL AND id IN (SELECT channel_id FROM channel_user WHERE user_id = $1))";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/e2ee/devices", get(list_devices).post(register_device))
        .route(
            "/api/e2ee/devices/{device_id}",
            axum::routing::patch(rename_device).delete(delete_device),
        )
        .route("/api/e2ee/keys/upload", post(upload_keys))
        .route("/api/e2ee/keys/query", post(query_keys))
        .route("/api/e2ee/keys/claim", post(claim_keys))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

// ---------------------------------------------------------------------------
// エラー

fn coded(status: StatusCode, code: &'static str, message: &str, extra: Value) -> AppError {
    AppError::Coded {
        status,
        code,
        message: message.to_owned(),
        extra: match extra {
            Value::Object(map) => map,
            _ => Map::new(),
        },
    }
}

fn limit_exceeded(limit: &str) -> AppError {
    limit_exceeded_with(limit, Map::new())
}

fn limit_exceeded_with(limit: &str, mut extra: Map<String, Value>) -> AppError {
    extra.insert("limit".into(), limit.into());
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_LIMIT_EXCEEDED",
        &format!("E2EE limit exceeded: {limit}"),
        Value::Object(extra),
    )
}

fn bad_signature() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_BAD_SIGNATURE",
        "Invalid signature",
        Value::Null,
    )
}

fn device_exists() -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_DEVICE_EXISTS",
        "The device ID is already used",
        Value::Null,
    )
}

fn key_exists() -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_KEY_EXISTS",
        "A different key with the same key ID already exists",
        Value::Null,
    )
}

fn device_not_found() -> AppError {
    AppError::NotFound("Device not found".into())
}

fn crypto_error(err: CryptoError) -> AppError {
    match err {
        CryptoError::MissingSignature { .. } | CryptoError::Signature(_) => bad_signature(),
        err => AppError::BadRequest(format!("Invalid key: {err}")),
    }
}

// ---------------------------------------------------------------------------
// 入力の検証（DB を使わない）

/// 小文字でハイフン区切りの UUID v4 か（`device_id` はクライアントが UUID v4 で作る）。
pub fn valid_device_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|u| {
        u.get_version_num() == 4
            && u.get_variant() == uuid::Variant::RFC4122
            && u.hyphenated().to_string() == id
    })
}

/// ワンタイム鍵とフォールバック鍵の `key_id`（vodozemac の `KeyId` をパディングなし base64 にしたもの）。
fn valid_key_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
}

/// フォールバック鍵の `key_id` の世代。vodozemac の `KeyId` は端末ごとに 1 ずつ増える u64 で、
/// `key_id` はそのビッグエンディアンの 8 バイトをパディングなし base64 にしたもの。
fn key_generation(key_id: &str) -> Result<u64, AppError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(key_id)
        .ok()
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map(u64::from_be_bytes)
        .ok_or_else(|| {
            AppError::BadRequest("fallback key_id must be an unpadded base64 64-bit counter".into())
        })
}

fn json_size(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

fn check_key_size(value: &Value) -> Result<(), AppError> {
    if json_size(value) > MAX_KEY_JSON_BYTES {
        return Err(limit_exceeded("key_size"));
    }
    Ok(())
}

/// `signatures` に入っている署名の件数（全ユーザー、全鍵 ID の合計）。
fn signature_count(value: &Value) -> usize {
    value
        .get("signatures")
        .and_then(Value::as_object)
        .map_or(0, |by_user| {
            by_user
                .values()
                .map(|keys| keys.as_object().map_or(1, Map::len))
                .sum()
        })
}

/// 検証済みのワンタイム鍵かフォールバック鍵。
#[derive(Debug, Clone)]
pub struct SignedKey {
    pub key_id: String,
    pub json: Value,
}

/// アップロードされたワンタイム鍵かフォールバック鍵を検証する（署名の検証は除く）。
///
/// キーは `signed_curve25519:<key_id>`、値は `{"key", "signatures"}`（フォールバック鍵は `"fallback": true` も）。
fn parse_keys(keys: &Map<String, Value>, fallback: bool) -> Result<Vec<SignedKey>, AppError> {
    if fallback && keys.len() > 1 {
        return Err(AppError::BadRequest(
            "Only one fallback key per algorithm can be uploaded".into(),
        ));
    }
    let mut parsed = Vec::with_capacity(keys.len());
    for (name, value) in keys {
        let key_id = name
            .strip_prefix(SIGNED_CURVE25519)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|id| valid_key_id(id))
            .ok_or_else(|| AppError::BadRequest(format!("Invalid key name: {name}")))?;
        if fallback {
            // 古いフォールバック鍵で巻き戻さないよう、世代を比べられる key_id だけを受け付ける。
            key_generation(key_id)?;
        }
        check_key_size(value)?;
        let key = value
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest(format!("{name} has no key")))?;
        Curve25519PublicKey::from_base64(key).map_err(|e| crypto_error(e.into()))?;
        let is_fallback = value.get("fallback") == Some(&Value::Bool(true));
        if is_fallback != fallback {
            return Err(AppError::BadRequest(format!(
                "{name} must {}have \"fallback\": true",
                if fallback { "" } else { "not " }
            )));
        }
        parsed.push(SignedKey {
            key_id: key_id.to_owned(),
            json: value.clone(),
        });
    }
    Ok(parsed)
}

/// 鍵が、端末の署名鍵 `ed25519:<device_id>` で署名されていることを確かめる。
fn verify_keys(
    keys: &[SignedKey],
    user_id: &str,
    device_id: &str,
    signing_key: &Ed25519PublicKey,
) -> Result<(), AppError> {
    let key_id = format!("ed25519:{device_id}");
    for key in keys {
        zoo_crypto::verify_json_signature(&key.json, user_id, &key_id, signing_key)
            .map_err(crypto_error)?;
    }
    Ok(())
}

fn check_signature_total<'a>(values: impl IntoIterator<Item = &'a Value>) -> Result<(), AppError> {
    let total: usize = values.into_iter().map(signature_count).sum();
    if total > MAX_SIGNATURES_PER_UPLOAD {
        return Err(limit_exceeded("signatures"));
    }
    Ok(())
}

fn check_display_name(name: &str) -> Result<(), AppError> {
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(AppError::BadRequest(format!(
            "display_name must be at most {MAX_DISPLAY_NAME_CHARS} characters"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DB の小さな補助

async fn rows(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<Vec<Value>, AppError> {
    crate::apps::query(tx, sql, params).await
}

async fn first(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<Option<Value>, AppError> {
    Ok(rows(tx, sql, params).await?.into_iter().next())
}

async fn exec(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<(), AppError> {
    let mut statement = toasty::sql::statement(sql);
    for value in params {
        statement = statement.bind(*value);
    }
    statement.exec(tx).await?;
    Ok(())
}

/// `users` の自分の行を `FOR SHARE` で読む。無効化されていれば 403。
///
/// `AuthUser` の確認はハンドラーとは別のトランザクションなので、そのあとで無効化がコミットされても、
/// ここで止める。先にこの行を読めば、無効化の `UPDATE` はこのトランザクションのコミットを待ち、
/// そのあとで（トリガーが）この書き込みの端末も削除する。
async fn lock_active_user(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<(), AppError> {
    let row = first(
        tx,
        "SELECT '{}'::text FROM users WHERE id = $1 AND NOT is_deactivated FOR SHARE",
        &[user_id],
    )
    .await?;
    row.map(|_| ())
        .ok_or_else(|| AppError::Forbidden("The account is deactivated".into()))
}

/// ユーザー単位のロック。端末の登録（台数の確認と登録）と 30 日の回収のバッチを直列にする。
async fn lock_user(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<(), AppError> {
    first(
        tx,
        "SELECT '{}'::text FROM pg_advisory_xact_lock(1, hashtext($1))",
        &[user_id],
    )
    .await?;
    Ok(())
}

/// 端末単位のロック。端末の削除、鍵のアップロード、アカウントの無効化（トリガー）、
/// to-device の送信（#355）で同じキーを使う。
pub async fn lock_device(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    device_id: &str,
) -> Result<(), AppError> {
    first(
        tx,
        "SELECT '{}'::text FROM pg_advisory_xact_lock(2, hashtext($1 || ':' || $2))",
        &[user_id, device_id],
    )
    .await?;
    Ok(())
}

/// `last_seen_at` を今にする。1 端末につき 5 分に 1 回まで。
pub async fn touch_last_seen(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    device_id: &str,
) -> Result<(), AppError> {
    exec(
        tx,
        "UPDATE e2ee_devices SET last_seen_at = now()
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL
           AND (last_seen_at IS NULL OR last_seen_at < now() - interval '5 minutes')",
        &[user_id, device_id],
    )
    .await
}

const DEVICE_JSON: &str = "json_build_object('device_id', d.device_id, 'display_name', d.display_name, 'curve25519_key', d.curve25519_key, 'ed25519_key', d.ed25519_key, 'device_keys', d.device_keys, 'created_at', d.created_at, 'last_seen_at', d.last_seen_at)";

async fn key_counts(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    device_id: &str,
) -> Result<Value, AppError> {
    first(
        tx,
        "SELECT json_build_object(
           'one_time_key_counts', json_build_object('signed_curve25519',
             (SELECT count(*) FROM e2ee_one_time_keys
                WHERE user_id = $1 AND device_id = $2 AND claimed_at IS NULL)),
           'unused_fallback_key_types', COALESCE((SELECT json_agg(algorithm ORDER BY algorithm)
             FROM e2ee_fallback_keys WHERE user_id = $1 AND device_id = $2 AND NOT used), '[]'::json)
         )::text",
        &[user_id, device_id],
    )
    .await?
    .ok_or_else(|| AppError::Internal(anyhow::anyhow!("key counts returned no row")))
}

/// 端末単位のロックを取ったあとで、ワンタイム鍵とフォールバック鍵を入れる。
///
/// 同じ `key_id` で同じ内容の再送は無視し、内容が違えば 409 `E2EE_KEY_EXISTS`。
/// 払い出し済みのワンタイム鍵（`claimed_at` あり）も行が残るので、主キーの衝突で入れ直さない。
/// フォールバック鍵は、今の鍵より新しい世代（`key_id`）のときだけ置き換える。
/// 入れたあとのワンタイム鍵とフォールバック鍵の合計が上限を超えるなら 400 `E2EE_LIMIT_EXCEEDED`。
async fn store_keys(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    device_id: &str,
    one_time_keys: &[SignedKey],
    fallback_key: Option<&SignedKey>,
) -> Result<(), AppError> {
    let new_keys = Value::Array(
        one_time_keys
            .iter()
            .map(|k| json!({"key_id": k.key_id, "key_json": k.json}))
            .collect(),
    )
    .to_string();
    let has_fallback = if fallback_key.is_some() {
        "true"
    } else {
        "false"
    };
    let check = first(
        tx,
        "WITH n AS (SELECT * FROM jsonb_to_recordset($3::jsonb) AS n(key_id text, key_json jsonb)),
         -- 置いてある鍵（払い出し済みを含む）と同じ key_id の鍵。
         seen AS (
           SELECT k.key_id, k.key_json FROM e2ee_one_time_keys k
             WHERE k.user_id = $1 AND k.device_id = $2 AND k.algorithm = 'signed_curve25519')
         SELECT json_build_object(
           'conflict', EXISTS (SELECT 1 FROM n JOIN seen ON seen.key_id = n.key_id
             WHERE seen.key_json <> n.key_json),
           'total', (SELECT count(*) FROM e2ee_one_time_keys k
               WHERE k.user_id = $1 AND k.device_id = $2 AND k.claimed_at IS NULL)
             + (SELECT count(*) FROM n WHERE NOT EXISTS (SELECT 1 FROM seen WHERE seen.key_id = n.key_id))
             + CASE WHEN $4::boolean OR EXISTS (SELECT 1 FROM e2ee_fallback_keys f
                 WHERE f.user_id = $1 AND f.device_id = $2) THEN 1 ELSE 0 END,
           'fallback', (SELECT json_build_object('key_id', f.key_id, 'same', f.key_json = $5::jsonb)
             FROM e2ee_fallback_keys f
             WHERE f.user_id = $1 AND f.device_id = $2 AND f.algorithm = 'signed_curve25519')
         )::text",
        &[
            user_id,
            device_id,
            &new_keys,
            has_fallback,
            &fallback_key
                .map(|k| k.json.to_string())
                .unwrap_or_else(|| "null".into()),
        ],
    )
    .await?
    .unwrap_or_default();
    if check["conflict"] == true {
        return Err(key_exists());
    }
    if check["total"].as_i64().unwrap_or(0) > MAX_KEYS_PER_DEVICE {
        return Err(limit_exceeded("one_time_keys"));
    }
    // 新しいフォールバック鍵に置き換えるか。端末単位のロックを持っているので、読んでから書くまでに変わらない。
    let replace_fallback = match (fallback_key, check["fallback"].as_object()) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(key), Some(current)) => {
            let current_id = current["key_id"].as_str().unwrap_or_default();
            if current_id == key.key_id {
                // 同じ鍵の再送は何もしない（used を戻さない）。同じ key_id で内容が違うのは 409。
                if current["same"] != true {
                    return Err(key_exists());
                }
                false
            } else {
                // 遅れて届いた古い鍵で、今の鍵を巻き戻さない。端末は今と 1 つ前の秘密鍵しか持たないので、
                // 古い鍵を払い出すと、宛先がセッションを開けない。
                key_generation(&key.key_id)? > key_generation(current_id)?
            }
        }
    };
    if !one_time_keys.is_empty() {
        // 払い出し済みの鍵も行が残るので、応答をなくした再送は主キーの衝突で入らない（同じ鍵を 2 回払い出さない）。
        // 払い出しがコミットする前でも、衝突した行の確定を待ってから判定するので、入り直らない。
        exec(
            tx,
            "INSERT INTO e2ee_one_time_keys (user_id, device_id, algorithm, key_id, key_json)
             SELECT $1, $2, 'signed_curve25519', n.key_id, n.key_json
             FROM jsonb_to_recordset($3::jsonb) AS n(key_id text, key_json jsonb)
             ON CONFLICT DO NOTHING",
            &[user_id, device_id, &new_keys],
        )
        .await?;
    }
    if let Some(key) = fallback_key.filter(|_| replace_fallback) {
        exec(
            tx,
            "INSERT INTO e2ee_fallback_keys (user_id, device_id, algorithm, key_id, key_json)
             VALUES ($1, $2, 'signed_curve25519', $3, $4::jsonb)
             ON CONFLICT (user_id, device_id, algorithm) DO UPDATE
             SET key_id = EXCLUDED.key_id, key_json = EXCLUDED.key_json, used = false, created_at = now()",
            &[user_id, device_id, &key.key_id, &key.json.to_string()],
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 端末

#[derive(Deserialize)]
pub struct RegisterRequest {
    device_keys: Value,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    one_time_keys: Map<String, Value>,
    #[serde(default)]
    fallback_keys: Map<String, Value>,
}

/// `POST /api/e2ee/devices`：端末を登録する。
async fn register_device(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let device_id = req
        .device_keys
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if !valid_device_id(&device_id) {
        return Err(AppError::BadRequest(
            "device_keys.device_id must be a lowercase UUID v4".into(),
        ));
    }
    check_display_name(&req.display_name)?;
    check_key_size(&req.device_keys)?;
    let one_time_keys = parse_keys(&req.one_time_keys, false)?;
    let fallback_keys = parse_keys(&req.fallback_keys, true)?;
    check_signature_total(
        std::iter::once(&req.device_keys)
            .chain(one_time_keys.iter().map(|k| &k.json))
            .chain(fallback_keys.iter().map(|k| &k.json)),
    )?;
    // user_id が呼び出したユーザーと一致すること、端末の自己署名を確かめる。
    let keys = zoo_crypto::verify_device_keys(&req.device_keys, &actor.id, &device_id)
        .map_err(crypto_error)?;
    verify_keys(&one_time_keys, &actor.id, &device_id, &keys.ed25519)?;
    verify_keys(&fallback_keys, &actor.id, &device_id, &keys.ed25519)?;
    let device_keys = req.device_keys.to_string();
    let curve25519 = keys.curve25519.to_base64();
    let ed25519 = keys.ed25519.to_base64();

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;

    // e2ee_devices と墓標を 1 つの文で確かめる。
    let existing = first(
        &mut tx,
        "SELECT json_build_object('deleted', deleted, 'same', same)::text FROM (
           SELECT d.deleted_at IS NOT NULL AS deleted, d.device_keys = $3::jsonb AS same
             FROM e2ee_devices d WHERE d.user_id = $1 AND d.device_id = $2
           UNION ALL
           SELECT true, false FROM e2ee_deleted_device_ids t WHERE t.user_id = $1 AND t.device_id = $2
         ) s",
        &[&actor.id, &device_id, &device_keys],
    )
    .await?;
    let inserted = existing.is_none();
    match existing {
        // 削除済み（墓標を含む）の device_id は再利用できない。内容が違う再送も拒否する。
        Some(row) if row["deleted"] == true || row["same"] != true => {
            return Err(device_exists());
        }
        // 削除されていない同じ内容の再送は成功として扱う（冪等）。
        Some(_) => {}
        None => {
            let recent = first(
                &mut tx,
                "SELECT json_build_object('count', count(*), 'retry_after',
                   GREATEST(1, ceil(extract(epoch FROM min(created_at) + interval '1 hour' - now())))::bigint)::text
                 FROM e2ee_devices WHERE user_id = $1 AND created_at > now() - interval '1 hour'",
                &[&actor.id],
            )
            .await?
            .unwrap_or_default();
            if recent["count"].as_i64().unwrap_or(0) >= MAX_REGISTRATIONS_PER_HOUR {
                return Err(coded(
                    StatusCode::TOO_MANY_REQUESTS,
                    "E2EE_RATE_LIMITED",
                    "Too many device registrations",
                    json!({"retry_after": recent["retry_after"].as_u64().unwrap_or(3600)}),
                ));
            }
            let devices = first(
                &mut tx,
                "SELECT json_build_object('count', count(*), 'stale', COALESCE(
                   json_agg(json_build_object('device_id', device_id, 'last_seen_at', last_seen_at)
                     ORDER BY last_seen_at, device_id)
                   FILTER (WHERE last_seen_at < now() - interval '30 days'), '[]'::json))::text
                 FROM e2ee_devices WHERE user_id = $1 AND deleted_at IS NULL",
                &[&actor.id],
            )
            .await?
            .unwrap_or_default();
            if devices["count"].as_i64().unwrap_or(0) >= MAX_DEVICES_PER_USER {
                // 古い端末は自動で削除しない。利用者に選んでもらう。
                let mut extra = Map::new();
                extra.insert("stale_devices".into(), devices["stale"].clone());
                return Err(limit_exceeded_with("devices", extra));
            }
            let inserted = toasty::sql::statement(
                "INSERT INTO e2ee_devices
                   (user_id, device_id, display_name, curve25519_key, ed25519_key, device_keys, last_seen_at)
                 VALUES ($1, $2, $3, $4, $5, $6::jsonb, now())",
            )
            .bind(actor.id.as_str())
            .bind(device_id.as_str())
            .bind(req.display_name.as_str())
            .bind(curve25519.as_str())
            .bind(ed25519.as_str())
            .bind(device_keys.as_str())
            .exec(&mut tx)
            .await;
            if let Err(err) = inserted {
                // 同じ識別鍵か署名鍵を持つ端末がすでにある。
                return Err(if AppError::is_unique_violation(&err, None) {
                    device_exists()
                } else {
                    err.into()
                });
            }
        }
    }

    // 端末の削除と同じロックのあとで deleted_at を読み直す。再送が既存の端末を読んだあとに削除が
    // コミットしていたら、削除済みの端末に鍵を入れずに 409 にする（keys/upload と同じ）。
    lock_device(&mut tx, &actor.id, &device_id).await?;
    first(
        &mut tx,
        "SELECT '{}'::text FROM e2ee_devices
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL",
        &[&actor.id, &device_id],
    )
    .await?
    .ok_or_else(device_exists)?;
    store_keys(
        &mut tx,
        &actor.id,
        &device_id,
        &one_time_keys,
        fallback_keys.first(),
    )
    .await?;
    let mut device = first(
        &mut tx,
        &format!(
            "SELECT {DEVICE_JSON}::text FROM e2ee_devices d WHERE d.user_id = $1 AND d.device_id = $2"
        ),
        &[&actor.id, &device_id],
    )
    .await?
    .ok_or_else(device_not_found)?;
    if let (Value::Object(device), Value::Object(counts)) = (
        &mut device,
        key_counts(&mut tx, &actor.id, &device_id).await?,
    ) {
        device.extend(counts);
    }
    // 端末の行を入れたときだけ txid を返す。既存の端末への再送（鍵の追加を含む）は Shape に流れる
    // 変更がないので、待っても進まない txid を返さない。
    let txid = if inserted {
        Some(get_current_txid(&mut tx).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(ApiResponse::new(device, txid)))
}

/// `GET /api/e2ee/devices`：自分の端末の一覧（削除済みを除く）。
async fn list_devices(
    State(state): State<AppState>,
    actor: AuthUser,
) -> Result<Json<Vec<Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let devices = rows(
        &mut tx,
        &format!(
            "SELECT {DEVICE_JSON}::text FROM e2ee_devices d
             WHERE d.user_id = $1 AND d.deleted_at IS NULL ORDER BY d.created_at, d.device_id"
        ),
        &[&actor.id],
    )
    .await?;
    Ok(Json(devices))
}

#[derive(Deserialize)]
pub struct RenameRequest {
    display_name: String,
}

/// `PATCH /api/e2ee/devices/{device_id}`：端末名を変える。自分の端末だけ。
async fn rename_device(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(device_id): Path<String>,
    Json(req): Json<RenameRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    check_display_name(&req.display_name)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let device = first(
        &mut tx,
        &format!(
            "UPDATE e2ee_devices d SET display_name = $3
             WHERE d.user_id = $1 AND d.device_id = $2 AND d.deleted_at IS NULL
             RETURNING {DEVICE_JSON}::text"
        ),
        &[&actor.id, &device_id, &req.display_name],
    )
    .await?
    .ok_or_else(device_not_found)?;
    tx.commit().await?;
    // 端末名は Shape の列に含めないので、観測できる変更はない。
    Ok(Json(ApiResponse::new(device, None)))
}

/// `DELETE /api/e2ee/devices/{device_id}`：自分の端末を論理削除する。冪等。
///
/// 削除は失効ではない（削除された端末は鍵を持ったまま）。`membership_epoch` を増やす処理は #357 で足す。
async fn delete_device(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(device_id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_device(&mut tx, &actor.id, &device_id).await?;
    let existing = first(
        &mut tx,
        "SELECT json_build_object('deleted', deleted)::text FROM (
           SELECT d.deleted_at IS NOT NULL AS deleted FROM e2ee_devices d
             WHERE d.user_id = $1 AND d.device_id = $2
           UNION ALL
           SELECT true FROM e2ee_deleted_device_ids t WHERE t.user_id = $1 AND t.device_id = $2
         ) s",
        &[&actor.id, &device_id],
    )
    .await?
    // 他人の端末も、存在を推測されないよう 404 にする。
    .ok_or_else(device_not_found)?;
    let data = json!({"device_id": device_id});
    if existing["deleted"] == true {
        // 削除済みの端末への再試行は 200（epoch も増やさない）。
        return Ok(Json(ApiResponse::new(data, None)));
    }
    // その端末に付いた自己署名だけを消す。device_id は (user_id, device_id) でしか一意でないので、
    // 別のユーザーの同じ device_id への署名は消さない。
    exec(
        &mut tx,
        "DELETE FROM e2ee_key_signatures
         WHERE signer_user_id = $1 AND target_user_id = $1 AND target_key_id = $2",
        &[&actor.id, &device_id],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM e2ee_one_time_keys WHERE user_id = $1 AND device_id = $2",
        &[&actor.id, &device_id],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM e2ee_fallback_keys WHERE user_id = $1 AND device_id = $2",
        &[&actor.id, &device_id],
    )
    .await?;
    exec(
        &mut tx,
        "UPDATE e2ee_devices SET deleted_at = now()
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL",
        &[&actor.id, &device_id],
    )
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(data, txid)))
}

// ---------------------------------------------------------------------------
// 鍵

#[derive(Deserialize)]
pub struct UploadRequest {
    device_id: String,
    #[serde(default)]
    one_time_keys: Map<String, Value>,
    #[serde(default)]
    fallback_keys: Map<String, Value>,
}

/// `POST /api/e2ee/keys/upload`：ワンタイム鍵とフォールバック鍵を足す。
async fn upload_keys(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(req): Json<UploadRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let one_time_keys = parse_keys(&req.one_time_keys, false)?;
    let fallback_keys = parse_keys(&req.fallback_keys, true)?;
    check_signature_total(
        one_time_keys
            .iter()
            .chain(fallback_keys.iter())
            .map(|k| &k.json),
    )?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    // 端末の署名鍵は変わらないので、ロックを取る前に読んで検証してよい。
    let signing_key = first(
        &mut tx,
        "SELECT to_json(ed25519_key)::text FROM e2ee_devices
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL",
        &[&actor.id, &req.device_id],
    )
    .await?
    .and_then(|key| key.as_str().map(str::to_owned))
    .ok_or_else(device_not_found)?;
    let signing_key = Ed25519PublicKey::from_base64(&signing_key)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("stored ed25519 key is invalid: {e}")))?;
    verify_keys(&one_time_keys, &actor.id, &req.device_id, &signing_key)?;
    verify_keys(&fallback_keys, &actor.id, &req.device_id, &signing_key)?;

    // 端末の削除と同じロックのあとで deleted_at を読み直す。削除のあとに鍵が入り直らないように。
    lock_device(&mut tx, &actor.id, &req.device_id).await?;
    first(
        &mut tx,
        "SELECT '{}'::text FROM e2ee_devices
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL",
        &[&actor.id, &req.device_id],
    )
    .await?
    .ok_or_else(device_not_found)?;
    store_keys(
        &mut tx,
        &actor.id,
        &req.device_id,
        &one_time_keys,
        fallback_keys.first(),
    )
    .await?;
    touch_last_seen(&mut tx, &actor.id, &req.device_id).await?;
    let counts = key_counts(&mut tx, &actor.id, &req.device_id).await?;
    tx.commit().await?;
    // 鍵は Shape で配らないので、観測できる変更はない。
    Ok(Json(ApiResponse::new(counts, None)))
}

#[derive(Deserialize)]
pub struct QueryRequest {
    user_ids: Vec<String>,
}

/// `POST /api/e2ee/keys/query`：ユーザーごとの端末鍵とクロス署名鍵を問い合わせる。
///
/// 対象は自分と、暗号化チャンネルを共有しているユーザー。範囲外のユーザーは応答から黙って除く。
/// 例外として、自分がピンか署名を持つ相手は、マスター鍵だけを返す。
async fn query_keys(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(req): Json<QueryRequest>,
) -> Result<Response, AppError> {
    if req.user_ids.len() > MAX_QUERY_USERS {
        return Err(limit_exceeded("query_users"));
    }
    let user_ids = Value::from(req.user_ids).to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let sql = format!(
        "WITH req AS (SELECT DISTINCT u AS user_id FROM jsonb_array_elements_text($2::jsonb) u),
         full_users AS (SELECT r.user_id FROM req r WHERE r.user_id = $1 OR r.user_id IN ({SHARED_USERS_SQL})),
         master_only AS (SELECT r.user_id FROM req r WHERE r.user_id NOT IN (SELECT user_id FROM full_users)
           AND (EXISTS (SELECT 1 FROM e2ee_tofu_pins p WHERE p.user_id = $1 AND p.target_user_id = r.user_id)
             OR EXISTS (SELECT 1 FROM e2ee_key_signatures s WHERE s.signer_user_id = $1 AND s.target_user_id = r.user_id))),
         visible AS (SELECT user_id FROM full_users UNION SELECT user_id FROM master_only)
         SELECT json_build_object(
           'device_keys', COALESCE((SELECT json_object_agg(f.user_id, (
               SELECT COALESCE(json_object_agg(d.device_id, d.device_keys), '{{}}'::json)
               FROM e2ee_devices d WHERE d.user_id = f.user_id AND d.deleted_at IS NULL))
             FROM full_users f), '{{}}'::json),
           'master_keys', COALESCE((SELECT json_object_agg(k.user_id, k.key_json) FROM e2ee_cross_signing_keys k
             WHERE k.usage = 'master' AND k.user_id IN (SELECT user_id FROM visible)), '{{}}'::json),
           'self_signing_keys', COALESCE((SELECT json_object_agg(k.user_id, k.key_json) FROM e2ee_cross_signing_keys k
             WHERE k.usage = 'self_signing' AND k.user_id IN (SELECT user_id FROM full_users)), '{{}}'::json),
           'user_signing_keys', COALESCE((SELECT json_object_agg(k.user_id, k.key_json) FROM e2ee_cross_signing_keys k
             WHERE k.usage = 'user_signing' AND k.user_id = $1 AND k.user_id IN (SELECT user_id FROM full_users)), '{{}}'::json),
           'signatures', COALESCE((SELECT json_agg(json_build_object(
               'signer_user_id', s.signer_user_id, 'signer_key_id', s.signer_key_id,
               'target_user_id', s.target_user_id, 'target_key_id', s.target_key_id, 'signature', s.signature)
               ORDER BY s.target_user_id, s.target_key_id, s.signer_user_id, s.signer_key_id)
             FROM e2ee_key_signatures s
             WHERE s.target_user_id IN (SELECT user_id FROM visible)
               AND (s.signer_user_id = $1
                 OR (s.signer_user_id = s.target_user_id AND s.target_user_id IN (SELECT user_id FROM full_users)))), '[]'::json),
           'tofu_pins', COALESCE((SELECT json_object_agg(p.target_user_id, p.pin_json) FROM e2ee_tofu_pins p
             WHERE p.user_id = $1 AND p.target_user_id IN (SELECT user_id FROM req)), '{{}}'::json)
         )::text"
    );
    let rows = toasty::sql::query(sql)
        .bind(actor.id.as_str())
        .bind(user_ids.as_str())
        .exec(&mut tx)
        .await?;
    let body = rows
        .into_iter()
        .find_map(|row| match row {
            toasty::stmt::Value::Record(rec) => match rec.fields.into_iter().next() {
                Some(toasty::stmt::Value::String(text)) => Some(text),
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("keys/query returned no row")))?;
    if body.len() > MAX_QUERY_RESPONSE_BYTES {
        return Err(limit_exceeded("response_size"));
    }
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body,
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct ClaimRequest {
    one_time_keys: BTreeMap<String, BTreeMap<String, String>>,
}

/// `POST /api/e2ee/keys/claim`：端末ごとにワンタイム鍵を 1 つ払い出す。なければフォールバック鍵を返す。
///
/// 呼び出したユーザーと宛先の端末の組ごとに、1 日に `MAX_CLAIMS_PER_DAY` 回まで。超えたらエラーにせず、
/// フォールバック鍵を返す。範囲外の端末は黙って応答から除く。
async fn claim_keys(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(req): Json<ClaimRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let requested: usize = req.one_time_keys.values().map(BTreeMap::len).sum();
    if requested > MAX_CLAIM_DEVICES {
        return Err(limit_exceeded("claim_devices"));
    }
    // (user_id, device_id) の昇順（バイト順。SQL の COLLATE "C" と同じ）。払い出しの回数の行と鍵の行を、
    // 並行した claim とアカウントの無効化（トリガー）で同じ順にロックし、行き詰まらないようにする。
    let targets: Vec<(&str, &str)> = req
        .one_time_keys
        .iter()
        .flat_map(|(user, devices)| {
            devices
                .iter()
                .filter(|(_, algorithm)| algorithm.as_str() == SIGNED_CURVE25519)
                .map(move |(device, _)| (user.as_str(), device.as_str()))
        })
        .collect();
    let mut result: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    if targets.is_empty() {
        return Ok(Json(ApiResponse::new(json!({"one_time_keys": {}}), None)));
    }
    let target_json = Value::Array(
        targets
            .iter()
            .map(|(user, device)| json!({"user_id": user, "device_id": device}))
            .collect(),
    )
    .to_string();

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let allowed: BTreeSet<(String, String)> = rows(
        &mut tx,
        &format!(
            "SELECT json_build_object('user_id', d.user_id, 'device_id', d.device_id)::text
             FROM jsonb_to_recordset($2::jsonb) AS t(user_id text, device_id text)
             JOIN e2ee_devices d ON d.user_id = t.user_id AND d.device_id = t.device_id AND d.deleted_at IS NULL
             WHERE d.user_id = $1 OR d.user_id IN ({SHARED_USERS_SQL})"
        ),
        &[&actor.id, &target_json],
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        Some((
            row["user_id"].as_str()?.to_owned(),
            row["device_id"].as_str()?.to_owned(),
        ))
    })
    .collect();
    let limit = MAX_CLAIMS_PER_DAY.to_string();
    for (user, device) in targets {
        if !allowed.contains(&(user.to_owned(), device.to_owned())) {
            continue;
        }
        // 要求の回数を原子的に数える。上限に達していれば行を返さない。
        let counted = first(
            &mut tx,
            "INSERT INTO e2ee_one_time_key_claim_counts
               (claimer_user_id, target_user_id, target_device_id, day, claimed)
             VALUES ($1, $2, $3, (now() AT TIME ZONE 'UTC')::date, 1)
             ON CONFLICT (claimer_user_id, target_user_id, target_device_id, day) DO UPDATE
             SET claimed = e2ee_one_time_key_claim_counts.claimed + 1
             WHERE e2ee_one_time_key_claim_counts.claimed < $4::integer
             RETURNING '{}'::text",
            &[&actor.id, user, device, &limit],
        )
        .await?
        .is_some();
        let mut key = None;
        if counted {
            // 並行した claim でも同じ鍵を 2 回払い出さない。ほかの claim がロックしている鍵は飛ばして、
            // 次の鍵を取る（UPDATE ... IN (SELECT ... SKIP LOCKED) の 1 文にすると、計画によっては
            // 残っている鍵があっても 0 行になるので、ロックと更新を分ける）。
            key = first(
                &mut tx,
                "SELECT json_build_object('key_id', key_id, 'key', key_json)::text
                 FROM e2ee_one_time_keys
                 WHERE user_id = $1 AND device_id = $2 AND algorithm = 'signed_curve25519'
                   AND claimed_at IS NULL
                 ORDER BY created_at, key_id LIMIT 1 FOR UPDATE SKIP LOCKED",
                &[user, device],
            )
            .await?;
            if let Some(key_id) = key.as_ref().and_then(|k| k["key_id"].as_str()) {
                // 行は消さずに払い出し済みにする。アップロードの再送が主キーで衝突し、入り直らないように。
                exec(
                    &mut tx,
                    "UPDATE e2ee_one_time_keys SET claimed_at = now()
                     WHERE user_id = $1 AND device_id = $2 AND algorithm = 'signed_curve25519' AND key_id = $3",
                    &[user, device, key_id],
                )
                .await?;
            }
        }
        if key.is_none() {
            key = first(
                &mut tx,
                "UPDATE e2ee_fallback_keys SET used = true
                 WHERE user_id = $1 AND device_id = $2 AND algorithm = 'signed_curve25519'
                 RETURNING json_build_object('key_id', key_id, 'key', key_json)::text",
                &[user, device],
            )
            .await?;
        }
        if let Some(key) = key {
            let name = format!(
                "{SIGNED_CURVE25519}:{}",
                key["key_id"].as_str().unwrap_or_default()
            );
            result
                .entry(user.to_owned())
                .or_default()
                .insert(device.to_owned(), json!({ name: key["key"] }));
        }
    }
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"one_time_keys": result}),
        None,
    )))
}

// ---------------------------------------------------------------------------
// 回収のバッチ

/// 削除から 30 日たった端末の行を墓標（`e2ee_deleted_device_ids`）に移して消し、
/// 当日（UTC）より前の払い出しの回数の行を消す。移した端末の数を返す。
///
/// ユーザーごとにユーザー単位のロックを取り、墓標の挿入と行の削除を 1 つのトランザクションで行う
/// （登録は両方の表を 1 つの文で確かめるので、移している途中の device_id も再利用されない）。
pub async fn sweep_deleted_devices(db: &toasty::Db) -> Result<u64, AppError> {
    let mut db = db.clone();
    let users = {
        let mut tx = db.transaction().await?;
        rows(
            &mut tx,
            "SELECT to_json(user_id)::text FROM (SELECT DISTINCT user_id FROM e2ee_devices
               WHERE deleted_at < now() - interval '30 days' LIMIT 1000) u",
            &[],
        )
        .await?
    };
    let mut moved = 0;
    for user in users {
        let Some(user) = user.as_str() else { continue };
        let mut tx = db.transaction().await?;
        lock_user(&mut tx, user).await?;
        let count = first(
            &mut tx,
            "WITH moved AS (
               DELETE FROM e2ee_devices WHERE user_id = $1 AND deleted_at < now() - interval '30 days'
               RETURNING user_id, device_id, deleted_at),
             signatures AS (
               DELETE FROM e2ee_key_signatures s USING moved m
               WHERE s.signer_user_id = m.user_id AND s.target_user_id = m.user_id AND s.target_key_id = m.device_id),
             tombstones AS (
               INSERT INTO e2ee_deleted_device_ids (user_id, device_id, deleted_at)
               SELECT user_id, device_id, deleted_at FROM moved ON CONFLICT DO NOTHING)
             SELECT count(*)::text FROM moved",
            &[user],
        )
        .await?;
        tx.commit().await?;
        moved += count.and_then(|c| c.as_u64()).unwrap_or(0);
    }
    let mut tx = db.transaction().await?;
    exec(
        &mut tx,
        "DELETE FROM e2ee_one_time_key_claim_counts WHERE day < (now() AT TIME ZONE 'UTC')::date",
        &[],
    )
    .await?;
    tx.commit().await?;
    Ok(moved)
}

/// 回収のバッチを 1 時間ごとに実行する。
pub async fn worker(state: AppState) {
    loop {
        if let Err(err) = sweep_deleted_devices(&state.db).await {
            tracing::warn!("E2EE device sweep failed: {err}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoo_crypto::DeviceAccount;

    fn account() -> DeviceAccount {
        DeviceAccount::new(&Uuid::new_v4().to_string(), &Uuid::new_v4().to_string()).unwrap()
    }

    #[test]
    fn device_ids_must_be_lowercase_uuid_v4() {
        let id = Uuid::new_v4().to_string();
        assert!(valid_device_id(&id));
        assert!(!valid_device_id(&id.to_uppercase()));
        assert!(!valid_device_id(&Uuid::new_v4().simple().to_string()));
        assert!(!valid_device_id("00000000-0000-1000-8000-000000000000"));
        assert!(!valid_device_id(""));
    }

    #[test]
    fn parses_and_verifies_signed_keys() {
        let mut account = account();
        account.generate_one_time_keys(3);
        account.generate_fallback_key();
        let otks = parse_keys(&account.one_time_keys(), false).unwrap();
        assert_eq!(otks.len(), 3);
        let fallback = parse_keys(&account.fallback_keys(), true).unwrap();
        assert_eq!(fallback.len(), 1);
        let ed = account.identity_keys().ed25519;
        verify_keys(&otks, account.user_id(), account.device_id(), &ed).unwrap();
        verify_keys(&fallback, account.user_id(), account.device_id(), &ed).unwrap();

        // 一方の種類を他方として送ると拒否する。
        assert!(parse_keys(&account.one_time_keys(), true).is_err());
        assert!(parse_keys(&account.fallback_keys(), false).is_err());

        // 別の端末の鍵では署名を検証できない。
        let other = account_ed25519();
        let err = verify_keys(&otks, account.user_id(), account.device_id(), &other).unwrap_err();
        assert!(matches!(
            err,
            AppError::Coded {
                code: "E2EE_BAD_SIGNATURE",
                ..
            }
        ));
    }

    fn account_ed25519() -> Ed25519PublicKey {
        account().identity_keys().ed25519
    }

    #[test]
    fn tampered_keys_fail_signature_checks() {
        let mut account = account();
        account.generate_one_time_keys(1);
        let mut keys = account.one_time_keys();
        let other = {
            let mut a = super::tests::account();
            a.generate_one_time_keys(1);
            a.one_time_keys().into_iter().next().unwrap().1["key"].clone()
        };
        for value in keys.values_mut() {
            value["key"] = other.clone();
        }
        let parsed = parse_keys(&keys, false).unwrap();
        let err = verify_keys(
            &parsed,
            account.user_id(),
            account.device_id(),
            &account.identity_keys().ed25519,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            AppError::Coded {
                code: "E2EE_BAD_SIGNATURE",
                ..
            }
        ));
    }

    #[test]
    fn rejects_bad_key_names_and_oversized_keys() {
        let key = json!({"key": "x", "signatures": {}});
        let mut map = Map::new();
        map.insert("curve25519:AAAA".into(), key.clone());
        assert!(parse_keys(&map, false).is_err());
        let mut map = Map::new();
        map.insert("signed_curve25519:".into(), key);
        assert!(parse_keys(&map, false).is_err());

        let big = json!({"key": "a".repeat(MAX_KEY_JSON_BYTES), "signatures": {}});
        let mut map = Map::new();
        map.insert("signed_curve25519:AAAA".into(), big);
        let err = parse_keys(&map, false).unwrap_err();
        assert!(matches!(
            err,
            AppError::Coded {
                code: "E2EE_LIMIT_EXCEEDED",
                ..
            }
        ));
    }

    #[test]
    fn counts_signatures_across_users_and_keys() {
        let value = json!({"signatures": {"a": {"k1": "s", "k2": "s"}, "b": {"k3": "s"}}});
        assert_eq!(signature_count(&value), 3);
        assert_eq!(signature_count(&json!({})), 0);
        let many: Vec<Value> = (0..=MAX_SIGNATURES_PER_UPLOAD)
            .map(|_| json!({"signatures": {"a": {"k": "s"}}}))
            .collect();
        assert!(check_signature_total(many.iter()).is_err());
        assert!(check_signature_total(many.iter().skip(1)).is_ok());
    }
}
