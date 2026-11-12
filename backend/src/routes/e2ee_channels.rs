//! 暗号化チャンネル（#357）。仕様は `docs/e2ee.md` の「暗号化チャンネル」「配布先のスナップショット」「メッセージ」
//! 「membership_epoch と送信の順序」。
//!
//! - `membership_epoch` を増やす処理と、送信と編集の epoch の照合
//! - `messages.ciphertext` の検証
//! - `GET /api/e2ee/channels/{channel_id}/recipients`（配布先のスナップショット）
//!
//! ロックの順は `routes/e2ee.rs` の先頭のとおり（`users` の行、ユーザー単位のロック、端末単位のロック、
//! `channels` の行（`id` の昇順）、`e2ee_devices` の行）。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    middleware,
    response::Response,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    auth::AuthUser,
    error::AppError,
    routes::e2ee::{coded, first, limit_exceeded, rows, touch_last_seen},
    state::AppState,
};

/// `messages.ciphertext` の JSON の大きさの上限（バイト）。
pub const MAX_CIPHERTEXT_BYTES: usize = 256 * 1024;
/// `recipients` の 1 ページのユーザー数の上限（既定値でもある）。
pub const RECIPIENTS_PAGE_USERS: usize = 50;
/// `recipients` の 1 ページの応答の大きさの上限（バイト）。
pub const MAX_RECIPIENTS_PAGE_BYTES: usize = 4 * 1024 * 1024;
/// 暗号化チャンネルのメンバー数の上限。
///
/// Megolm セッションは送信者が全メンバーの全端末（1 人 20 台まで）に to-device で配るので、
/// 200 人 × 20 台 = 4000 台が 1 回の配布の上限になる（`keys/claim` の 1000 台ずつで 4 回）。
pub const MAX_ENCRYPTED_CHANNEL_MEMBERS: i64 = 200;
/// `messages.ciphertext.algorithm` に入れる値。
pub const MEGOLM_ALGORITHM: &str = "m.megolm.v1.aes-sha2";

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/e2ee/channels/{channel_id}/recipients",
            get(get_recipients),
        )
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

pub fn stale_epoch(current: i64) -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_STALE_EPOCH",
        "The membership epoch has changed",
        json!({"current_epoch": current}),
    )
}

pub fn invalid_epoch() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_INVALID_EPOCH",
        "The epoch is newer than the channel's membership epoch",
        Value::Null,
    )
}

pub fn stale_revision(current: i64) -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_STALE_REVISION",
        "The message has been edited",
        json!({"current_revision": current}),
    )
}

pub fn plaintext_forbidden() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_PLAINTEXT_FORBIDDEN",
        "Plaintext and encrypted fields cannot be mixed in this channel",
        Value::Null,
    )
}

pub fn ciphertext_required() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_CIPHERTEXT_REQUIRED",
        "An encrypted channel requires ciphertext",
        Value::Null,
    )
}

pub fn attachments_forbidden() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_ATTACHMENTS_FORBIDDEN",
        "Attachments are not allowed in an encrypted channel",
        Value::Null,
    )
}

pub fn encrypted_immutable() -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_ENCRYPTED_IMMUTABLE",
        "The encryption of a channel cannot be changed",
        Value::Null,
    )
}

pub fn encrypted_mismatch() -> AppError {
    coded(
        StatusCode::CONFLICT,
        "E2EE_ENCRYPTED_MISMATCH",
        "The encrypted direct message is not encrypted",
        Value::Null,
    )
}

pub fn public_forbidden() -> AppError {
    coded(
        StatusCode::BAD_REQUEST,
        "E2EE_PUBLIC_FORBIDDEN",
        "Only private channels can be encrypted",
        Value::Null,
    )
}

pub fn apps_forbidden() -> AppError {
    coded(
        StatusCode::FORBIDDEN,
        "E2EE_APPS_FORBIDDEN",
        "Apps cannot use encrypted channels",
        Value::Null,
    )
}

pub fn add_forbidden() -> AppError {
    coded(
        StatusCode::FORBIDDEN,
        "E2EE_ADD_FORBIDDEN",
        "Only current members can add members to an encrypted channel",
        Value::Null,
    )
}

pub fn disabled() -> AppError {
    coded(
        StatusCode::FORBIDDEN,
        "E2EE_DISABLED",
        "Encrypted channels are disabled on this server",
        Value::Null,
    )
}

/// `encrypted: true` の作成を受け付けるか。App（委任トークンを含む）は設定によらず拒否する
/// （判定の順は、App かどうか、設定）。
pub fn check_encrypted_creation(state: &AppState, from_app: bool) -> Result<(), AppError> {
    if from_app {
        return Err(apps_forbidden());
    }
    if !state.config.e2ee_encrypted_channels_enabled {
        return Err(disabled());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// membership_epoch

/// そのユーザーが参加しているすべての暗号化チャンネル（削除済みを除く）の `membership_epoch` を 1 増やす。
///
/// 行は `id` の昇順にまとめて `FOR NO KEY UPDATE` でロックしてから更新する（DB の関数
/// `e2ee_bump_membership_epochs`。アカウントの無効化のトリガーと同じ）。端末単位のロックより後、
/// `e2ee_devices` の行より前に呼ぶ。
pub async fn bump_user_epochs(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<(), AppError> {
    first(
        tx,
        "SELECT '{}'::text FROM (SELECT e2ee_bump_membership_epochs($1)) s",
        &[user_id],
    )
    .await?;
    Ok(())
}

/// 1 つのチャンネルの `membership_epoch` を 1 増やす（暗号化チャンネルだけ）。
/// `channel_user` の行が実際に変わったとき（参加、追加、退出、キック、DM への再参加）に呼ぶ。
pub async fn bump_channel_epoch(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
) -> Result<(), AppError> {
    let mut statement = toasty::sql::statement(
        "UPDATE channels SET membership_epoch = membership_epoch + 1
         WHERE id = $1 AND encrypted AND deleted_at IS NULL",
    );
    statement = statement.bind(channel_id);
    statement.exec(tx).await?;
    Ok(())
}

/// 暗号化チャンネルのメンバー数の上限を確かめる（追加の前に呼ぶ）。
pub async fn check_member_limit(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    adding: i64,
) -> Result<(), AppError> {
    let row = first(
        tx,
        "SELECT json_build_object('count', count(*))::text FROM channel_user WHERE channel_id = $1",
        &[channel_id],
    )
    .await?
    .unwrap_or_default();
    if row["count"].as_i64().unwrap_or(0) + adding > MAX_ENCRYPTED_CHANNEL_MEMBERS {
        return Err(limit_exceeded("members"));
    }
    Ok(())
}

/// 送信と編集のトランザクションで、チャンネルの行を `FOR SHARE` で読む。
/// `membership_epoch` を増やす処理（`UPDATE`）と直列になる。
pub struct ChannelEpoch {
    pub encrypted: bool,
    pub membership_epoch: i64,
}

pub async fn lock_channel_epoch(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
) -> Result<ChannelEpoch, AppError> {
    let row = first(
        tx,
        "SELECT json_build_object('encrypted', encrypted, 'epoch', membership_epoch)::text
         FROM channels WHERE id = $1 FOR SHARE",
        &[channel_id],
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Channel not found: {channel_id}")))?;
    Ok(ChannelEpoch {
        encrypted: row["encrypted"] == true,
        membership_epoch: row["epoch"].as_i64().unwrap_or(0),
    })
}

/// 送信と編集の `epoch` を、今の `membership_epoch` と比べる。
pub fn check_epoch(sent: i64, current: i64) -> Result<(), AppError> {
    match sent.cmp(&current) {
        std::cmp::Ordering::Less => Err(stale_epoch(current)),
        std::cmp::Ordering::Greater => Err(invalid_epoch()),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// messages.ciphertext

/// 検証した `ciphertext` の送信端末。
#[derive(Debug)]
pub struct CiphertextSender {
    pub device_id: String,
    pub sender_key: String,
}

/// `ciphertext` の形を確かめる（DB を使わない）。
pub fn check_ciphertext_shape(ciphertext: &Value) -> Result<CiphertextSender, AppError> {
    let object = ciphertext
        .as_object()
        .ok_or_else(|| AppError::BadRequest("ciphertext must be an object".into()))?;
    if crate::routes::e2ee::json_size(ciphertext) > MAX_CIPHERTEXT_BYTES {
        return Err(limit_exceeded("ciphertext_size"));
    }
    if object.get("algorithm").and_then(Value::as_str) != Some(MEGOLM_ALGORITHM) {
        return Err(AppError::BadRequest(format!(
            "ciphertext.algorithm must be {MEGOLM_ALGORITHM}"
        )));
    }
    let field = |name: &str| -> Result<String, AppError> {
        object
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| AppError::BadRequest(format!("ciphertext.{name} is required")))
    };
    field("session_id")?;
    field("ciphertext")?;
    Ok(CiphertextSender {
        device_id: field("device_id")?,
        sender_key: field("sender_key")?,
    })
}

/// `ciphertext` の送信端末が、送信者の削除されていない端末で、`sender_key` がその端末の識別鍵と一致することを
/// 確かめ、端末の `last_seen_at` を更新する（5 分に 1 回まで）。チャンネルの行のロックのあとで呼ぶ。
pub async fn check_sender_device(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    sender: &CiphertextSender,
) -> Result<(), AppError> {
    first(
        tx,
        "SELECT '{}'::text FROM e2ee_devices
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL AND curve25519_key = $3",
        &[user_id, &sender.device_id, &sender.sender_key],
    )
    .await?
    .ok_or_else(|| {
        AppError::BadRequest(
            "ciphertext.device_id and sender_key must be one of your current devices".into(),
        )
    })?;
    touch_last_seen(tx, user_id, &sender.device_id).await
}

// ---------------------------------------------------------------------------
// 配布先のスナップショット

#[derive(Deserialize)]
pub struct RecipientsQuery {
    limit: Option<usize>,
    after: Option<String>,
    epoch: Option<i64>,
}

/// `GET /api/e2ee/channels/{channel_id}/recipients`：配布先のスナップショット。
///
/// 各ページは 1 つの `REPEATABLE READ READ ONLY` のトランザクションで作る。続きのページは、最初のページの
/// `membership_epoch` を `epoch` に入れて取り、今の epoch と違えば 409 `E2EE_STALE_EPOCH`。
async fn get_recipients(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<RecipientsQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(RECIPIENTS_PAGE_USERS);
    if limit == 0 || limit > RECIPIENTS_PAGE_USERS {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {RECIPIENTS_PAGE_USERS}"
        )));
    }
    let after = query.after.unwrap_or_default();
    if !after.is_empty() && query.epoch.is_none() {
        return Err(AppError::BadRequest(
            "epoch is required when after is given".into(),
        ));
    }

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // BEGIN の直後（最初の文）に、スナップショットを固定する。
    toasty::sql::statement("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .exec(&mut tx)
        .await?;
    let not_found = || AppError::NotFound(format!("Channel not found with id: {channel_id}"));
    let channel = first(
        &mut tx,
        "SELECT json_build_object('epoch', c.membership_epoch, 'member', EXISTS (
           SELECT 1 FROM channel_user cu WHERE cu.channel_id = c.id AND cu.user_id = $2))::text
         FROM channels c WHERE c.id = $1 AND c.encrypted AND c.deleted_at IS NULL",
        &[&channel_id, &actor.id],
    )
    .await?
    .ok_or_else(not_found)?;
    if channel["member"] != true {
        return Err(not_found());
    }
    let epoch = channel["epoch"].as_i64().unwrap_or(0);
    if let Some(sent) = query.epoch
        && sent != epoch
    {
        return Err(stale_epoch(epoch));
    }

    // 1 人分ずつ組み立て、続きがあるかを知るため 1 人多く読む。
    let fetch = (limit + 1).to_string();
    let users = rows(
        &mut tx,
        "SELECT json_build_object(
           'user_id', m.user_id,
           'device_keys', COALESCE((SELECT json_object_agg(d.device_id, d.device_keys ORDER BY d.device_id)
             FROM e2ee_devices d WHERE d.user_id = m.user_id AND d.deleted_at IS NULL), '{}'::json),
           'master_key', (SELECT k.key_json FROM e2ee_cross_signing_keys k
             WHERE k.user_id = m.user_id AND k.usage = 'master'),
           'self_signing_key', (SELECT k.key_json FROM e2ee_cross_signing_keys k
             WHERE k.user_id = m.user_id AND k.usage = 'self_signing'),
           'user_signing_key', (SELECT k.key_json FROM e2ee_cross_signing_keys k
             WHERE k.user_id = m.user_id AND k.user_id = $2 AND k.usage = 'user_signing'),
           -- 配布先の判定が使う、呼び出したユーザーの署名と、そのユーザー自身の署名だけ。
           'signatures', COALESCE((SELECT json_agg(json_build_object(
               'signer_user_id', s.signer_user_id, 'signer_key_id', s.signer_key_id,
               'target_user_id', s.target_user_id, 'target_key_id', s.target_key_id, 'signature', s.signature)
               ORDER BY s.target_key_id, s.signer_user_id, s.signer_key_id)
             FROM e2ee_key_signatures s
             WHERE s.target_user_id = m.user_id
               AND (s.signer_user_id = $2 OR s.signer_user_id = s.target_user_id)), '[]'::json),
           'tofu_pin', (SELECT p.pin_json FROM e2ee_tofu_pins p
             WHERE p.user_id = $2 AND p.target_user_id = m.user_id)
         )::text
         FROM (SELECT cu.user_id FROM channel_user cu
               WHERE cu.channel_id = $1 AND cu.user_id COLLATE \"C\" > $3
               ORDER BY cu.user_id COLLATE \"C\" LIMIT $4::int) m
         ORDER BY m.user_id COLLATE \"C\"",
        &[&channel_id, &actor.id, &after, &fetch],
    )
    .await?;
    tx.commit().await?;

    Ok(Json(build_recipients_page(
        &channel_id,
        epoch,
        users,
        limit,
        MAX_RECIPIENTS_PAGE_BYTES,
    )?))
}

/// 1 人分の行（`user_id` の昇順。`limit` より 1 人多く読んだもの）から、1 ページの応答を組み立てる。
///
/// 応答が `max_bytes` を超えるときは、入る人数で切って `next_after` を返す。1 人分だけで超えるときは
/// 400 `E2EE_LIMIT_EXCEEDED`（`response_size`）。
pub fn build_recipients_page(
    channel_id: &str,
    epoch: i64,
    users: Vec<Value>,
    limit: usize,
    max_bytes: usize,
) -> Result<Value, AppError> {
    let mut user_ids = Vec::new();
    let mut device_keys = Map::new();
    let mut master_keys = Map::new();
    let mut self_signing_keys = Map::new();
    let mut user_signing_keys = Map::new();
    let mut signatures = Vec::new();
    let mut tofu_pins = Map::new();

    let base = json!({
        "channel_id": channel_id, "membership_epoch": epoch, "user_ids": [],
        "device_keys": {}, "master_keys": {}, "self_signing_keys": {}, "user_signing_keys": {},
        "signatures": [], "tofu_pins": {}, "next_after": Value::Null,
    });
    // next_after に入る user_id の分も見込む。
    let mut size = crate::routes::e2ee::json_size(&base);
    let has_more_rows = users.len() > limit;
    let mut truncated = false;
    for user in users.into_iter().take(limit) {
        let Some(user_id) = user["user_id"].as_str().map(str::to_owned) else {
            continue;
        };
        // 1 人分の大きさ（キー名と区切りを含めて多めに見積もる）。
        let user_size = crate::routes::e2ee::json_size(&user) + 2 * user_id.len() * 6 + 64;
        if user_size + crate::routes::e2ee::json_size(&base) > max_bytes {
            if user_ids.is_empty() {
                return Err(limit_exceeded("response_size"));
            }
            truncated = true;
            break;
        }
        if size + user_size > max_bytes {
            truncated = true;
            break;
        }
        size += user_size;
        device_keys.insert(user_id.clone(), user["device_keys"].clone());
        for (map, key) in [
            (&mut master_keys, "master_key"),
            (&mut self_signing_keys, "self_signing_key"),
            (&mut user_signing_keys, "user_signing_key"),
            (&mut tofu_pins, "tofu_pin"),
        ] {
            if !user[key].is_null() {
                map.insert(user_id.clone(), user[key].clone());
            }
        }
        if let Value::Array(items) = &user["signatures"] {
            signatures.extend(items.iter().cloned());
        }
        user_ids.push(user_id);
    }
    let next_after = if (has_more_rows || truncated) && !user_ids.is_empty() {
        Value::from(user_ids.last().cloned())
    } else {
        Value::Null
    };
    Ok(json!({
        "channel_id": channel_id,
        "membership_epoch": epoch,
        "user_ids": user_ids,
        "device_keys": device_keys,
        "master_keys": master_keys,
        "self_signing_keys": self_signing_keys,
        "user_signing_keys": user_signing_keys,
        "signatures": signatures,
        "tofu_pins": tofu_pins,
        "next_after": next_after,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str, pad: usize) -> Value {
        json!({
            "user_id": id,
            "device_keys": {"dev": {"pad": "x".repeat(pad)}},
            "master_key": null, "self_signing_key": null, "user_signing_key": null,
            "signatures": [], "tofu_pin": null,
        })
    }

    #[test]
    fn epoch_comparison() {
        assert!(check_epoch(3, 3).is_ok());
        let err = check_epoch(2, 3).unwrap_err();
        assert!(
            matches!(err, AppError::Coded { code: "E2EE_STALE_EPOCH", ref extra, .. } if extra["current_epoch"] == 3)
        );
        assert!(matches!(
            check_epoch(4, 3).unwrap_err(),
            AppError::Coded {
                code: "E2EE_INVALID_EPOCH",
                ..
            }
        ));
    }

    #[test]
    fn ciphertext_shape() {
        let ok = json!({"algorithm": MEGOLM_ALGORITHM, "sender_key": "sk", "device_id": "d",
            "session_id": "s", "ciphertext": "c"});
        let sender = check_ciphertext_shape(&ok).unwrap();
        assert_eq!(sender.device_id, "d");
        assert_eq!(sender.sender_key, "sk");
        let mut wrong = ok.clone();
        wrong["algorithm"] = "m.olm.v1.curve25519-aes-sha2".into();
        assert!(check_ciphertext_shape(&wrong).is_err());
        let mut missing = ok.clone();
        missing.as_object_mut().unwrap().remove("session_id");
        assert!(check_ciphertext_shape(&missing).is_err());
        assert!(check_ciphertext_shape(&json!("text")).is_err());
        let mut big = ok;
        big["ciphertext"] = "a".repeat(MAX_CIPHERTEXT_BYTES).into();
        assert!(matches!(
            check_ciphertext_shape(&big).unwrap_err(),
            AppError::Coded { code: "E2EE_LIMIT_EXCEEDED", ref extra, .. } if extra["limit"] == "ciphertext_size"
        ));
    }

    #[test]
    fn recipients_page_without_more() {
        let page =
            build_recipients_page("c", 4, vec![user("a", 1), user("b", 1)], 50, 1 << 20).unwrap();
        assert_eq!(page["user_ids"], json!(["a", "b"]));
        assert_eq!(page["membership_epoch"], 4);
        assert!(page["next_after"].is_null());
        assert!(page["master_keys"].as_object().unwrap().is_empty());
    }

    #[test]
    fn recipients_page_with_more_rows() {
        let page = build_recipients_page(
            "c",
            0,
            vec![user("a", 1), user("b", 1), user("c", 1)],
            2,
            1 << 20,
        )
        .unwrap();
        assert_eq!(page["user_ids"], json!(["a", "b"]));
        assert_eq!(page["next_after"], "b");
    }

    #[test]
    fn recipients_page_cut_by_size() {
        let page = build_recipients_page(
            "c",
            0,
            vec![user("a", 600), user("b", 600), user("c", 600)],
            50,
            2000,
        )
        .unwrap();
        assert_eq!(page["user_ids"], json!(["a", "b"]));
        assert_eq!(page["next_after"], "b");
        assert!(crate::routes::e2ee::json_size(&page) <= 2000);
    }

    #[test]
    fn recipients_single_user_too_large() {
        assert!(matches!(
            build_recipients_page("c", 0, vec![user("a", 2000)], 50, 1500).unwrap_err(),
            AppError::Coded { code: "E2EE_LIMIT_EXCEEDED", ref extra, .. } if extra["limit"] == "response_size"
        ));
    }
}
