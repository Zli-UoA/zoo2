//! E2EE の鍵バックアップの API（#356）。仕様は `docs/e2ee.md` の「鍵バックアップ」。
//!
//! 今の版は、そのユーザーが作った版のうち番号が最も大きいもの。それを消してあれば今の版はない
//! （古い版に戻らない）。版の作成と削除はユーザー単位のロック、アップロードは同じキーの共有ロックを取ってから
//! 今の版を読むので、版を切り替えたあとに古い版へのアップロードは通らない。

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
use zoo_crypto::Curve25519PublicKey;

use crate::{
    auth::AuthUser,
    error::AppError,
    routes::{
        e2ee::{
            check_key_size, coded, crypto_error, exec, first, json_size, limit_exceeded,
            lock_active_user, lock_user, rows,
        },
        e2ee_cross_signing::{present, verify_signed_by},
        e2ee_reauth::reauth,
        e2ee_secret_storage::{check_expected_default_key, default_key},
        response::ApiResponse,
    },
    state::AppState,
};

/// 鍵バックアップの方式。
pub const BACKUP_ALGORITHM: &str = "zoo.megolm_backup.v1.curve25519-aes-sha2";
/// 1 回のアップロードと、1 ページの取得のセッションの数。
pub const MAX_BACKUP_SESSIONS: usize = 1000;
/// 1 つのセッションの `session_data` の JSON の大きさの上限（バイト）。
pub const MAX_SESSION_DATA_BYTES: usize = 8 * 1024;
/// アップロードの要求の本文の上限（1000 件 × 8 KiB と、残りの列）。
const MAX_UPLOAD_BODY_BYTES: usize = 10 * 1024 * 1024;
/// `channel_id`、`sender_key`、`session_id` の長さの上限。
const MAX_ID_CHARS: usize = 128;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/e2ee/key_backup/versions",
            get(list_versions).post(create_version),
        )
        .route(
            "/api/e2ee/key_backup/versions/current",
            get(current_version),
        )
        .route(
            "/api/e2ee/key_backup/versions/{version}",
            get(get_version).delete(delete_version),
        )
        .route(
            "/api/e2ee/key_backup/keys",
            get(get_sessions)
                .post(upload_sessions)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_BYTES)),
        )
}

fn wrong_version(current: Option<i64>) -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_WRONG_BACKUP_VERSION",
        "The backup version is not the current one",
        json!({"current_version": current.map(|v| v.to_string())}),
    )
}

fn version_not_found() -> AppError {
    AppError::NotFound("Backup version not found".into())
}

/// `auth_data` に署名した版の番号（`auth_data.version`）。方式と番号を入れていない古い形式なら `None`。
///
/// クライアントは、方式と、サーバーが振る版の番号を `auth_data` に入れて署名する（docs/e2ee.md の
/// 「鍵バックアップ」）。古い版の `auth_data` を別の番号の版として見せる巻き戻しを、クライアントが
/// 見分けられるようにするため。古い形式は、最初の版（版 1）だけ受け付ける。
fn signed_version(auth_data: &Value) -> Result<Option<String>, AppError> {
    match (auth_data.get("algorithm"), auth_data.get("version")) {
        (None, None) => Ok(None),
        (Some(Value::String(algorithm)), Some(Value::String(version)))
            if algorithm == BACKUP_ALGORITHM =>
        {
            Ok(Some(version.clone()))
        }
        _ => Err(AppError::BadRequest(
            "auth_data.algorithm and auth_data.version must name this backup algorithm and version"
                .into(),
        )),
    }
}

/// 版 `version` を作る、または更新する `auth_data` の、署名した番号を確かめる。番号が違えば 409
/// `E2EE_WRONG_BACKUP_VERSION`（別の端末が先に版を作ったなど。クライアントは版を読み直す）、
/// 版 2 以降に古い形式を使えば 400。
fn check_signed_version(
    auth_data: &Value,
    version: i64,
    current: Option<i64>,
) -> Result<(), AppError> {
    match signed_version(auth_data)? {
        Some(signed) if signed == version.to_string() => Ok(()),
        Some(_) => Err(wrong_version(current)),
        None if version == 1 => Ok(()),
        None => Err(AppError::BadRequest(
            "auth_data.algorithm and auth_data.version are required after the first version"
                .into(),
        )),
    }
}

/// 版の番号（10 進の文字列）を読む。
fn parse_version(version: &str) -> Result<i64, AppError> {
    version
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0 && v.to_string() == version)
        .ok_or_else(version_not_found)
}

/// 今の版（番号が最大の版。消してあれば `None`）。
async fn current(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Option<Value>, AppError> {
    first(
        tx,
        "SELECT json_build_object('version', version, 'auth_data', auth_data)::text
         FROM e2ee_key_backup_versions WHERE user_id = $1 AND deleted_at IS NULL
           AND version = (SELECT max(version) FROM e2ee_key_backup_versions WHERE user_id = $1)",
        &[user_id],
    )
    .await
}

#[derive(Deserialize)]
pub struct CreateVersionRequest {
    algorithm: String,
    auth_data: Value,
    #[serde(default, deserialize_with = "present")]
    expected_current_version: Option<Value>,
    /// 前提にした既定の SSSS 鍵（初回設定、リセット、失効。#360、#362）。省略したら照合しない。
    #[serde(default)]
    expected_default_key_id: Option<String>,
    /// 前提にした `secrets_revision`（失効。#362）。端末だけの失効は SSSS の鍵を変えないので、別の端末が
    /// 秘密だけを書き換えたことは、これで見分ける。省略したら照合しない。
    #[serde(default)]
    expected_secrets_revision: Option<i64>,
}

/// `POST /api/e2ee/key_backup/versions`：新しい版を作り、今の版にする。
async fn create_version(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreateVersionRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    if req.algorithm != BACKUP_ALGORITHM {
        return Err(AppError::BadRequest("Unsupported backup algorithm".into()));
    }
    check_key_size(&req.auth_data)?;
    let public_key = req
        .auth_data
        .get("public_key")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("auth_data.public_key is required".into()))?
        .to_owned();
    let parsed =
        Curve25519PublicKey::from_base64(&public_key).map_err(|e| crypto_error(e.into()))?;
    if parsed.to_base64() != public_key {
        return Err(AppError::BadRequest(
            "auth_data.public_key must be unpadded standard base64".into(),
        ));
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
    let now = current(&mut tx, &actor.id).await?;
    let now_version = now.as_ref().and_then(|v| v["version"].as_i64());

    // 今の版と同じ公開鍵：応答をなくした再送は、新しい版を作らずに今の版を返す（再認証も epoch の増加もない）。
    // 署名だけが違う（マスター鍵での署名し直し）なら、今のマスター鍵で検証して auth_data を更新する。
    if let Some(now) = &now
        && now["auth_data"]["public_key"].as_str() == Some(public_key.as_str())
    {
        let version = now_version.unwrap_or_default();
        if now["auth_data"] != req.auth_data {
            check_signed_version(&req.auth_data, version, now_version)?;
            verify_auth_data(&mut tx, &actor.id, &req.auth_data).await?;
            exec(
                &mut tx,
                "UPDATE e2ee_key_backup_versions SET auth_data = $3::jsonb
                 WHERE user_id = $1 AND version = $2::bigint",
                &[&actor.id, &version.to_string(), &req.auth_data.to_string()],
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(Json(ApiResponse::new(
            json!({"version": version.to_string()}),
            None,
        )));
    }

    let expected = match &req.expected_current_version {
        None if now.is_some() => {
            return Err(AppError::BadRequest(
                "expected_current_version is required unless there is no current version".into(),
            ));
        }
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v.parse::<i64>().map_err(|_| wrong_version(now_version))?),
        Some(_) => {
            return Err(AppError::BadRequest(
                "expected_current_version must be a string or null".into(),
            ));
        }
    };
    if expected != now_version {
        return Err(wrong_version(now_version));
    }
    verify_auth_data(&mut tx, &actor.id, &req.auth_data).await?;
    let history = first(
        &mut tx,
        "SELECT json_build_object('max', max(version),
           'reused', bool_or(auth_data ->> 'public_key' = $2))::text
         FROM e2ee_key_backup_versions WHERE user_id = $1",
        &[&actor.id, &public_key],
    )
    .await?
    .unwrap_or_default();
    if history["reused"] == true {
        // 漏れたバックアップ鍵の古い auth_data を送り直して、今の版に戻せないようにする。
        return Err(coded(
            StatusCode::BAD_REQUEST,
            "E2EE_KEY_REUSED",
            "The backup key was used by a previous version",
            Value::Null,
        ));
    }
    let max = history["max"].as_i64();
    let version = max.unwrap_or(0) + 1;
    check_signed_version(&req.auth_data, version, now_version)?;
    if max.is_some() {
        // 最初の版でなければ（消した版を含めて過去に版があれば）、直前の再認証が要る。
        reauth.require()?;
        // 切り替えより前の版に入ったセッションを、切り替えのあとの送信に使わせないため、参加している
        // 暗号化チャンネルの membership_epoch を増やす。
        crate::routes::e2ee_channels::bump_user_epochs(&mut tx, &actor.id).await?;
    }
    exec(
        &mut tx,
        "INSERT INTO e2ee_key_backup_versions (user_id, version, algorithm, auth_data)
         VALUES ($1, $2::bigint, $3, $4::jsonb)",
        &[
            &actor.id,
            &version.to_string(),
            BACKUP_ALGORITHM,
            &req.auth_data.to_string(),
        ],
    )
    .await?;
    // epoch を増やしたときは Channels Shape に変更が流れるので、txid を返す。
    let txid = if max.is_some() {
        Some(crate::db::get_current_txid(&mut tx).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"version": version.to_string()}),
        txid,
    )))
}

/// `auth_data` に今のマスター鍵の署名があるか。
async fn verify_auth_data(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    auth_data: &Value,
) -> Result<(), AppError> {
    let master = first(
        tx,
        "SELECT to_json(public_key)::text FROM e2ee_cross_signing_keys
         WHERE user_id = $1 AND usage = 'master'",
        &[user_id],
    )
    .await?
    .and_then(|k| k.as_str().map(str::to_owned))
    .ok_or_else(|| AppError::BadRequest("A master key is required before a backup".into()))?;
    verify_signed_by(auth_data, user_id, &master)
}

/// `GET /api/e2ee/key_backup/versions`：すべての版（消した版を含む）を番号の昇順で返す。
async fn list_versions(
    State(state): State<AppState>,
    actor: AuthUser,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let versions = rows(
        &mut tx,
        "SELECT json_build_object('version', version::text, 'algorithm', algorithm,
           'auth_data', auth_data, 'deleted', deleted_at IS NOT NULL)::text
         FROM e2ee_key_backup_versions WHERE user_id = $1 ORDER BY version",
        &[&actor.id],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"versions": versions})))
}

/// `GET /api/e2ee/key_backup/versions/current`：今の版。なければ 404（古い版を返さない）。
async fn current_version(
    State(state): State<AppState>,
    actor: AuthUser,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let version = current(&mut tx, &actor.id)
        .await?
        .and_then(|v| v["version"].as_i64())
        .ok_or_else(version_not_found)?;
    let body = version_json(&mut tx, &actor.id, version, true).await?;
    tx.commit().await?;
    Ok(Json(body))
}

async fn version_json(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    version: i64,
    with_count: bool,
) -> Result<Value, AppError> {
    first(
        tx,
        "SELECT json_build_object('version', v.version::text, 'algorithm', v.algorithm,
           'auth_data', v.auth_data, 'count', CASE WHEN $3::boolean THEN
             (SELECT count(*) FROM e2ee_key_backup_sessions s WHERE s.user_id = v.user_id AND s.version = v.version)
           END)::text
         FROM e2ee_key_backup_versions v
         WHERE v.user_id = $1 AND v.version = $2::bigint AND v.deleted_at IS NULL",
        &[user_id, &version.to_string(), if with_count { "true" } else { "false" }],
    )
    .await?
    .map(|mut v| {
        if !with_count && let Value::Object(map) = &mut v {
            map.remove("count");
        }
        v
    })
    .ok_or_else(version_not_found)
}

/// `GET /api/e2ee/key_backup/versions/{version}`：消していない版。消した版とない版は 404。
async fn get_version(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(version): Path<String>,
) -> Result<Json<Value>, AppError> {
    let version = parse_version(&version)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let body = version_json(&mut tx, &actor.id, version, false).await?;
    tx.commit().await?;
    Ok(Json(body))
}

/// `DELETE /api/e2ee/key_backup/versions/{version}`：版のセッションを消し、版に `deleted_at` を入れる。
/// 今の版を消すときは直前の再認証が要る。
async fn delete_version(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: HeaderMap,
    Path(version): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let version = parse_version(&version)?;
    let reauth = reauth(&state, &headers, &actor).await?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    lock_user(&mut tx, &actor.id).await?;
    let target = first(
        &mut tx,
        "SELECT json_build_object('current',
           version = (SELECT max(version) FROM e2ee_key_backup_versions WHERE user_id = $1))::text
         FROM e2ee_key_backup_versions
         WHERE user_id = $1 AND version = $2::bigint AND deleted_at IS NULL",
        &[&actor.id, &version.to_string()],
    )
    .await?
    .ok_or_else(version_not_found)?;
    if target["current"] == true {
        reauth.require()?;
    }
    exec(
        &mut tx,
        "DELETE FROM e2ee_key_backup_sessions WHERE user_id = $1 AND version = $2::bigint",
        &[&actor.id, &version.to_string()],
    )
    .await?;
    exec(
        &mut tx,
        "UPDATE e2ee_key_backup_versions SET deleted_at = now()
         WHERE user_id = $1 AND version = $2::bigint",
        &[&actor.id, &version.to_string()],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"version": version.to_string()}),
        None,
    )))
}

// ---------------------------------------------------------------------------
// セッション

#[derive(Deserialize, Clone)]
pub struct BackupSession {
    channel_id: String,
    sender_key: String,
    session_id: String,
    first_message_index: i64,
    forwarded_count: i64,
    is_verified: bool,
    session_data: Value,
}

impl BackupSession {
    fn validate(&self) -> Result<(), AppError> {
        for (field, value) in [
            ("channel_id", &self.channel_id),
            ("sender_key", &self.sender_key),
            ("session_id", &self.session_id),
        ] {
            if value.is_empty() || value.len() > MAX_ID_CHARS {
                return Err(AppError::BadRequest(format!(
                    "{field} must be 1 to {MAX_ID_CHARS} characters"
                )));
            }
        }
        if !(0..=i64::from(i32::MAX)).contains(&self.first_message_index)
            || !(0..=i64::from(i32::MAX)).contains(&self.forwarded_count)
        {
            return Err(AppError::BadRequest(
                "first_message_index and forwarded_count must be non-negative 32-bit integers"
                    .into(),
            ));
        }
        if json_size(&self.session_data) > MAX_SESSION_DATA_BYTES {
            return Err(limit_exceeded("session_size"));
        }
        let fields_ok = ["ephemeral", "ciphertext", "mac"]
            .iter()
            .all(|f| self.session_data.get(*f).and_then(Value::as_str).is_some());
        if !fields_ok {
            return Err(AppError::BadRequest(
                "session_data must have ephemeral, ciphertext and mac".into(),
            ));
        }
        Ok(())
    }

    fn key(&self) -> (String, String, String) {
        (
            self.channel_id.clone(),
            self.sender_key.clone(),
            self.session_id.clone(),
        )
    }

    /// サーバーが残す方を決める順（`is_verified` が true、`first_message_index` が小さい、
    /// `forwarded_count` が小さい）。大きいほうがよい。
    fn rank(&self) -> (bool, i64, i64) {
        (
            self.is_verified,
            -self.first_message_index,
            -self.forwarded_count,
        )
    }

    fn json(&self) -> Value {
        json!({
            "channel_id": self.channel_id, "sender_key": self.sender_key, "session_id": self.session_id,
            "first_message_index": self.first_message_index, "forwarded_count": self.forwarded_count,
            "is_verified": self.is_verified, "session_data": self.session_data,
        })
    }
}

#[derive(Deserialize)]
pub struct UploadQuery {
    version: Option<String>,
    secrets_revision: Option<String>,
}

#[derive(Deserialize)]
pub struct UploadRequest {
    sessions: Vec<BackupSession>,
}

/// `POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`：今の版にセッションを上げる。
async fn upload_sessions(
    State(state): State<AppState>,
    actor: AuthUser,
    Query(query): Query<UploadQuery>,
    Json(req): Json<UploadRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let version = query
        .version
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("version is required".into()))?;
    // 起動し直していない端末が、失効の前の鍵で上げ続けないよう、最後に解錠したときの値を必須にする。
    let secrets_revision = query
        .secrets_revision
        .as_deref()
        .and_then(|r| r.parse::<i64>().ok())
        .ok_or_else(|| AppError::BadRequest("secrets_revision is required".into()))?;
    if req.sessions.len() > MAX_BACKUP_SESSIONS {
        return Err(limit_exceeded("backup_sessions"));
    }
    // 同じ要求の中の同じセッションは、よいほうだけを残す（1 つの INSERT で同じ行を 2 回更新できないため）。
    let mut best: BTreeMap<(String, String, String), BackupSession> = BTreeMap::new();
    for session in &req.sessions {
        session.validate()?;
        match best.get(&session.key()) {
            Some(kept) if kept.rank() >= session.rank() => {}
            _ => {
                best.insert(session.key(), session.clone());
            }
        }
    }

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 無効化されたユーザーの書き込みは 403（AuthUser の確認のあとに無効化がコミットしても
    // 書き込まない）。ロックの順は全体の規則どおり、users の行、ユーザー単位のロック。
    lock_active_user(&mut tx, &actor.id).await?;
    // 版の作成と削除（同じキーの排他ロック）とだけ直列にし、アップロードどうしは並行させる。
    first(
        &mut tx,
        "SELECT '{}'::text FROM pg_advisory_xact_lock_shared(1, hashtext($1))",
        &[&actor.id],
    )
    .await?;
    let now = current(&mut tx, &actor.id)
        .await?
        .and_then(|v| v["version"].as_i64());
    if now.is_none() || now.map(|v| v.to_string()).as_deref() != Some(version) {
        return Err(wrong_version(now));
    }
    let ssss = default_key(&mut tx, &actor.id).await?;
    if ssss.as_ref().and_then(|k| k["secrets_revision"].as_i64()) != Some(secrets_revision) {
        return Err(coded(
            StatusCode::CONFLICT,
            "E2EE_SSSS_CONFLICT",
            "The secret storage has changed",
            json!({
                "default_key_id": ssss.as_ref().map_or(Value::Null, |k| k["key_id"].clone()),
                "secrets_revision": ssss.as_ref().map_or(Value::Null, |k| k["secrets_revision"].clone()),
            }),
        ));
    }
    if !best.is_empty() {
        // 読んでから書くと、並行した同じセッションのアップロードがよい行を上書きしたり、主キーの衝突（500）に
        // なったりするので、1 つの文で決める。
        exec(
            &mut tx,
            "INSERT INTO e2ee_key_backup_sessions AS cur
               (user_id, version, channel_id, sender_key, session_id, first_message_index,
                forwarded_count, is_verified, session_data)
             SELECT $1, $2::bigint, s.channel_id, s.sender_key, s.session_id, s.first_message_index,
                    s.forwarded_count, s.is_verified, s.session_data
             FROM jsonb_to_recordset($3::jsonb) AS s(channel_id text, sender_key text, session_id text,
               first_message_index integer, forwarded_count integer, is_verified boolean, session_data jsonb)
             ON CONFLICT (user_id, version, channel_id, sender_key, session_id) DO UPDATE
             SET first_message_index = EXCLUDED.first_message_index,
                 forwarded_count = EXCLUDED.forwarded_count,
                 is_verified = EXCLUDED.is_verified,
                 session_data = EXCLUDED.session_data,
                 updated_at = now()
             WHERE (EXCLUDED.is_verified::int, -EXCLUDED.first_message_index, -EXCLUDED.forwarded_count)
                 > (cur.is_verified::int, -cur.first_message_index, -cur.forwarded_count)",
            &[
                &actor.id,
                version,
                &Value::Array(best.values().map(BackupSession::json).collect()).to_string(),
            ],
        )
        .await?;
    }
    let count = first(
        &mut tx,
        "SELECT count(*)::text FROM e2ee_key_backup_sessions WHERE user_id = $1 AND version = $2::bigint",
        &[&actor.id, version],
    )
    .await?
    .unwrap_or(Value::Null);
    tx.commit().await?;
    Ok(Json(ApiResponse::new(json!({"count": count}), None)))
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    version: Option<String>,
    channel_id: Option<String>,
    after: Option<String>,
}

fn encode_cursor(row: &Value) -> String {
    let key = json!([row["channel_id"], row["sender_key"], row["session_id"]]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.to_string())
}

fn decode_cursor(cursor: &str) -> Result<[String; 3], AppError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<[String; 3]>(&bytes).ok())
        .ok_or_else(|| AppError::BadRequest("Invalid cursor".into()))
}

/// `GET /api/e2ee/key_backup/keys?version=<版>&channel_id=<任意>&after=<任意>`：
/// `(channel_id, sender_key, session_id)` の昇順に 1000 件ずつ返す。
async fn get_sessions(
    State(state): State<AppState>,
    actor: AuthUser,
    Query(query): Query<SessionsQuery>,
) -> Result<Json<Value>, AppError> {
    let version = parse_version(
        query
            .version
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("version is required".into()))?,
    )?;
    let after = query.after.as_deref().map(decode_cursor).transpose()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    version_json(&mut tx, &actor.id, version, false).await?;
    let (c, s, i) = after.as_ref().map_or(("", "", ""), |a| {
        (a[0].as_str(), a[1].as_str(), a[2].as_str())
    });
    // 並び順とカーソルの比較は、照合順序によらないバイト順（COLLATE "C"）にそろえる。
    let mut sessions = rows(
        &mut tx,
        "SELECT json_build_object('channel_id', channel_id, 'sender_key', sender_key,
           'session_id', session_id, 'first_message_index', first_message_index,
           'forwarded_count', forwarded_count, 'is_verified', is_verified,
           'session_data', session_data)::text
         FROM e2ee_key_backup_sessions
         WHERE user_id = $1 AND version = $2::bigint
           AND ($3 = '' OR channel_id = $3)
           AND (NOT $4::boolean OR (channel_id COLLATE \"C\", sender_key COLLATE \"C\", session_id COLLATE \"C\")
                > ($5 COLLATE \"C\", $6 COLLATE \"C\", $7 COLLATE \"C\"))
         ORDER BY channel_id COLLATE \"C\", sender_key COLLATE \"C\", session_id COLLATE \"C\"
         LIMIT $8::bigint",
        &[
            &actor.id,
            &version.to_string(),
            query.channel_id.as_deref().unwrap_or(""),
            if after.is_some() { "true" } else { "false" },
            c,
            s,
            i,
            &(MAX_BACKUP_SESSIONS + 1).to_string(),
        ],
    )
    .await?;
    tx.commit().await?;
    let next = if sessions.len() > MAX_BACKUP_SESSIONS {
        sessions.truncate(MAX_BACKUP_SESSIONS);
        sessions.last().map(encode_cursor)
    } else {
        None
    };
    Ok(Json(json!({"sessions": sessions, "next": next})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(verified: bool, index: i64, forwarded: i64) -> BackupSession {
        BackupSession {
            channel_id: "c".into(),
            sender_key: "k".into(),
            session_id: "s".into(),
            first_message_index: index,
            forwarded_count: forwarded,
            is_verified: verified,
            session_data: json!({"ephemeral": "e", "ciphertext": "c", "mac": "m"}),
        }
    }

    #[test]
    fn ranks_sessions_like_matrix() {
        assert!(session(true, 10, 5).rank() > session(false, 0, 0).rank());
        assert!(session(false, 0, 5).rank() > session(false, 1, 0).rank());
        assert!(session(false, 0, 0).rank() > session(false, 0, 1).rank());
    }

    #[test]
    fn cursors_round_trip_and_reject_garbage() {
        let row = json!({"channel_id": "a", "sender_key": "b/+", "session_id": "c"});
        assert_eq!(
            decode_cursor(&encode_cursor(&row)).unwrap(),
            ["a".to_owned(), "b/+".to_owned(), "c".to_owned()]
        );
        assert!(decode_cursor("!!").is_err());
        assert!(decode_cursor("e30").is_err());
    }

    #[test]
    fn versions_are_canonical_positive_integers() {
        assert_eq!(parse_version("12").unwrap(), 12);
        for bad in ["0", "-1", "012", "1.0", "x", ""] {
            assert!(parse_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validates_session_fields_and_size() {
        session(true, 0, 0).validate().unwrap();
        assert!(session(true, -1, 0).validate().is_err());
        let mut big = session(true, 0, 0);
        big.session_data["ciphertext"] = json!("a".repeat(MAX_SESSION_DATA_BYTES));
        assert!(matches!(
            big.validate(),
            Err(AppError::Coded {
                code: "E2EE_LIMIT_EXCEEDED",
                ..
            })
        ));
        let mut missing = session(true, 0, 0);
        missing.session_data = json!({"ephemeral": "e"});
        assert!(missing.validate().is_err());
    }
}
