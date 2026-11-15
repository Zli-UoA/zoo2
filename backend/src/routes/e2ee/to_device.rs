//! to-device メッセージの送信と受信確認（#355）。仕様は `docs/e2ee.md` の「to-device メッセージ」。
//!
//! 受信は宛先の端末ごとの Shape（`/api/shapes/e2ee_to_device`。`routes/shapes.rs`）で受け取る。
//! ロックの順は親モジュールの先頭に書いた全体の順に従う（`users` の行、端末単位のロックを宛先の昇順、
//! `e2ee_devices` の行（外部キーの `FOR KEY SHARE`））。

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{SHARED_USERS_SQL, exec, first, limit_exceeded, lock_active_user, lock_device, rows};
use crate::{
    auth::AuthUser, db::get_current_txid, error::AppError, routes::response::ApiResponse,
    state::AppState,
};

/// 1 回の送信の宛先の端末の数の上限。超える送信者は `txn_id` を変えて分割する。
pub const MAX_TO_DEVICE_RECIPIENTS: usize = 100;
/// 1 件の `content` の大きさの上限（バイト）。
pub const MAX_TO_DEVICE_CONTENT_BYTES: usize = 64 * 1024;
/// 宛先の端末ごとの、受信確認されていないメッセージの上限。
pub const MAX_TO_DEVICE_QUEUE: i64 = 1000;
/// `to_device/ack` で 1 回に消す ID の上限。
pub const MAX_TO_DEVICE_ACK_IDS: usize = 1000;
/// 送信のルートのボディの上限（宛先 100 × `content` 64 KiB が入る大きさ。既定の 2 MB では足りない）。
pub const TO_DEVICE_BODY_LIMIT: usize = 8 * 1024 * 1024;
/// 送信の記録（`e2ee_to_device_txns`）を残す期間。ack 後の同じ `txn_id` の再送を冪等にする。
pub const TO_DEVICE_TXN_RETENTION: &str = "30 days";
/// 初期版で受け付ける外側の `type`。
const TO_DEVICE_TYPE: &str = "m.room.encrypted";

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/e2ee/to_device",
            post(send_to_device).layer(DefaultBodyLimit::max(TO_DEVICE_BODY_LIMIT)),
        )
        .route("/api/e2ee/to_device/ack", post(ack_to_device))
}

/// 小文字でハイフン区切りの UUID か（`txn_id`。版は問わない）。
fn valid_txn_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)
}

/// 呼び出したユーザーの、削除されていない端末か。
pub(crate) async fn own_active_device(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    device_id: &str,
) -> Result<bool, AppError> {
    Ok(first(
        tx,
        "SELECT '{}'::text FROM e2ee_devices
         WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL",
        &[user_id, device_id],
    )
    .await?
    .is_some())
}

#[derive(Deserialize)]
pub struct SendRequest {
    device_id: String,
    txn_id: String,
    #[serde(rename = "type")]
    kind: String,
    /// `user_id` → `device_id` → `content`。`BTreeMap` なので `(user_id, device_id)` のバイト順に並ぶ。
    messages: BTreeMap<String, BTreeMap<String, Value>>,
}

/// 検証済みの宛先と、その `content`（JSON の文字列）。`(user_id, device_id)` の昇順に並べる。
fn parse_recipients(req: &SendRequest) -> Result<Vec<(&str, &str, String)>, AppError> {
    if req.kind != TO_DEVICE_TYPE {
        return Err(AppError::BadRequest(format!(
            "type must be {TO_DEVICE_TYPE}"
        )));
    }
    if !valid_txn_id(&req.txn_id) {
        return Err(AppError::BadRequest(
            "txn_id must be a lowercase hyphenated UUID".into(),
        ));
    }
    let count: usize = req.messages.values().map(BTreeMap::len).sum();
    if count > MAX_TO_DEVICE_RECIPIENTS {
        return Err(limit_exceeded("to_device_recipients"));
    }
    let mut recipients = Vec::with_capacity(count);
    for (user, devices) in &req.messages {
        for (device, content) in devices {
            if !content.is_object() {
                return Err(AppError::BadRequest(
                    "Each to-device content must be a JSON object".into(),
                ));
            }
            let content = content.to_string();
            if content.len() > MAX_TO_DEVICE_CONTENT_BYTES {
                return Err(limit_exceeded("to_device_content"));
            }
            recipients.push((user.as_str(), device.as_str(), content));
        }
    }
    Ok(recipients)
}

/// `POST /api/e2ee/to_device`：to-device メッセージを複数の宛先の端末に送る。
///
/// - 宛先にできるのは、送信者自身の端末と、送信者と暗号化チャンネルを共有しているユーザーの端末だけ。
///   範囲外の宛先と、存在しない端末や削除済みの端末への宛先は、知らせずに捨てる。
/// - 宛先の端末ごとの端末単位のロックを `(user_id, device_id)` の昇順で取り、件数の確認と挿入を直列にする。
///   ロックのあとで `deleted_at` を読み直すので、端末の削除と並行しても削除済みの端末に入らない。
/// - 受信確認されていないメッセージが上限に達した宛先は入れず、応答の `undelivered` で返す。
/// - 同じ `(送信ユーザー, 送信端末, txn_id, 宛先)` の再送は、キューにあっても ack 後でも 1 件にまとめる。
pub(crate) async fn send_to_device(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(req): Json<SendRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let recipients = parse_recipients(&req)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_active_user(&mut tx, &actor.id).await?;
    if !own_active_device(&mut tx, &actor.id, &req.device_id).await? {
        return Err(super::device_not_found());
    }
    if recipients.is_empty() {
        tx.commit().await?;
        return Ok(Json(ApiResponse::new(json!({"undelivered": []}), None)));
    }

    // 宛先にできる端末（範囲内で、削除されていないもの）。端末の Shape と同じ範囲。
    let targets = Value::Array(
        recipients
            .iter()
            .map(|(user, device, _)| json!({"user_id": user, "device_id": device}))
            .collect(),
    )
    .to_string();
    let allowed: std::collections::BTreeSet<(String, String)> = rows(
        &mut tx,
        &format!(
            "SELECT json_build_object('user_id', d.user_id, 'device_id', d.device_id)::text
             FROM jsonb_to_recordset($2::jsonb) AS t(user_id text, device_id text)
             JOIN e2ee_devices d ON d.user_id = t.user_id AND d.device_id = t.device_id AND d.deleted_at IS NULL
             WHERE d.user_id = $1 OR d.user_id IN ({SHARED_USERS_SQL})"
        ),
        &[&actor.id, &targets],
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

    let limit = MAX_TO_DEVICE_QUEUE.to_string();
    let mut undelivered = Vec::new();
    let mut queued_any = false;
    for (user, device, content) in &recipients {
        if !allowed.contains(&(user.to_string(), device.to_string())) {
            continue;
        }
        // 端末の削除とアカウントの無効化と同じキー。宛先の昇順で取る（recipients は昇順）。
        lock_device(&mut tx, user, device).await?;
        // ロックのあとの文なので、ロックを待つあいだにコミットされた削除と送信が見える。
        let status = first(
            &mut tx,
            "SELECT json_build_object(
               'alive', EXISTS (SELECT 1 FROM e2ee_devices
                 WHERE user_id = $1 AND device_id = $2 AND deleted_at IS NULL),
               'sent', EXISTS (SELECT 1 FROM e2ee_to_device_txns
                   WHERE sender_user_id = $3 AND sender_device_id = $4 AND txn_id = $5
                     AND recipient_user_id = $1 AND recipient_device_id = $2)
                 OR EXISTS (SELECT 1 FROM e2ee_to_device_messages
                   WHERE sender_user_id = $3 AND sender_device_id = $4 AND txn_id = $5
                     AND recipient_user_id = $1 AND recipient_device_id = $2),
               'full', (SELECT count(*) FROM (SELECT 1 FROM e2ee_to_device_messages
                   WHERE recipient_user_id = $1 AND recipient_device_id = $2 LIMIT $6::integer) q)
                 >= $6::integer
             )::text",
            &[
                *user,
                *device,
                &actor.id,
                &req.device_id,
                &req.txn_id,
                &limit,
            ],
        )
        .await?
        .unwrap_or_default();
        if status["alive"] != true || status["sent"] == true {
            // 削除済みの端末は知らせずに捨てる。同じ txn_id の再送は成功として扱う（入れ直さない）。
            continue;
        }
        if status["full"] == true {
            undelivered.push(json!({"user_id": user, "device_id": device}));
            continue;
        }
        // 送信の記録は、キューに入れた宛先にだけ書く（undelivered の宛先を同じ txn_id で送り直せるように）。
        exec(
            &mut tx,
            "WITH m AS (
               INSERT INTO e2ee_to_device_messages
                 (recipient_user_id, recipient_device_id, sender_user_id, sender_device_id, txn_id, type, content)
               VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
               ON CONFLICT DO NOTHING)
             INSERT INTO e2ee_to_device_txns
               (sender_user_id, sender_device_id, txn_id, recipient_user_id, recipient_device_id)
             VALUES ($3, $4, $5, $1, $2)
             ON CONFLICT DO NOTHING",
            &[
                *user,
                *device,
                &actor.id,
                &req.device_id,
                &req.txn_id,
                TO_DEVICE_TYPE,
                content.as_str(),
            ],
        )
        .await?;
        queued_any = true;
    }
    // 宛先の Shape に流れる行を入れたときだけ txid を返す。
    let txid = if queued_any {
        Some(get_current_txid(&mut tx).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"undelivered": undelivered}),
        txid,
    )))
}

#[derive(Deserialize)]
pub struct AckRequest {
    device_id: String,
    /// メッセージの ID。Shape は bigint を文字列で配るので、文字列と数のどちらも受け付ける。
    ids: Vec<Value>,
}

fn parse_ids(ids: &[Value]) -> Result<Vec<i64>, AppError> {
    if ids.len() > MAX_TO_DEVICE_ACK_IDS {
        return Err(limit_exceeded("to_device_ack"));
    }
    ids.iter()
        .map(|id| match id {
            Value::String(s) => s.parse().ok(),
            Value::Number(n) => n.as_i64(),
            _ => None,
        })
        .map(|id| id.ok_or_else(|| AppError::BadRequest("ids must be integers".into())))
        .collect()
}

/// `POST /api/e2ee/to_device/ack`：処理済みのメッセージを ID の一覧で消す。
///
/// 呼び出した端末宛ての行だけを消す（ID は推測できるので、他人宛ての行を消させない）。
/// すでに消えた ID と、他の端末宛ての ID は黙って無視する。
pub(crate) async fn ack_to_device(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(req): Json<AckRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let ids = Value::from(parse_ids(&req.ids)?).to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if !own_active_device(&mut tx, &actor.id, &req.device_id).await? {
        return Err(super::device_not_found());
    }
    let deleted = first(
        &mut tx,
        "WITH d AS (
           DELETE FROM e2ee_to_device_messages
           WHERE recipient_user_id = $1 AND recipient_device_id = $2
             AND id = ANY (SELECT jsonb_array_elements_text($3::jsonb)::bigint)
           RETURNING 1)
         SELECT count(*)::text FROM d",
        &[&actor.id, &req.device_id, &ids],
    )
    .await?
    .and_then(|n| n.as_u64())
    .unwrap_or(0);
    super::touch_last_seen(&mut tx, &actor.id, &req.device_id).await?;
    let txid = if deleted > 0 {
        Some(get_current_txid(&mut tx).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(ApiResponse::new(json!({"deleted": deleted}), txid)))
}

/// 30 日より前の送信の記録を消す。消した行の数を返す。
pub async fn sweep_txns(db: &toasty::Db) -> Result<u64, AppError> {
    let mut db = db.clone();
    let mut tx = db.transaction().await?;
    let count = first(
        &mut tx,
        &format!(
            "WITH d AS (DELETE FROM e2ee_to_device_txns
               WHERE created_at < now() - interval '{TO_DEVICE_TXN_RETENTION}' RETURNING 1)
             SELECT count(*)::text FROM d"
        ),
        &[],
    )
    .await?
    .and_then(|n| n.as_u64())
    .unwrap_or(0);
    tx.commit().await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(messages: Value) -> SendRequest {
        serde_json::from_value(json!({
            "device_id": "11111111-1111-4111-8111-111111111111",
            "txn_id": "22222222-2222-4222-8222-222222222222",
            "type": "m.room.encrypted",
            "messages": messages,
        }))
        .unwrap()
    }

    fn limit(err: AppError) -> String {
        match err {
            AppError::Coded { extra, .. } => extra["limit"].as_str().unwrap().to_owned(),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn txn_ids_must_be_lowercase_uuids() {
        assert!(valid_txn_id("22222222-2222-4222-8222-222222222222"));
        assert!(valid_txn_id("01890a5d-ac96-774b-bcce-b302099a8057"));
        assert!(!valid_txn_id("22222222-2222-4222-8222-22222222222A"));
        assert!(!valid_txn_id("22222222222242228222222222222222"));
        assert!(!valid_txn_id("txn-1"));
    }

    #[test]
    fn recipients_are_sorted_and_limited() {
        let req = request(json!({
            "b": {"2": {}, "1": {}},
            "a": {"9": {"algorithm": "x"}},
        }));
        let parsed: Vec<(&str, &str)> = parse_recipients(&req)
            .unwrap()
            .into_iter()
            .map(|(u, d, _)| (u, d))
            .collect();
        assert_eq!(parsed, vec![("a", "9"), ("b", "1"), ("b", "2")]);

        let devices = |n: usize| -> Value {
            Value::Object((0..n).map(|i| (i.to_string(), json!({}))).collect())
        };
        assert!(parse_recipients(&request(json!({"a": devices(60), "b": devices(40)}))).is_ok());
        let err = parse_recipients(&request(json!({"a": devices(60), "b": devices(41)})))
            .err()
            .unwrap();
        assert_eq!(limit(err), "to_device_recipients");
    }

    #[test]
    fn content_must_be_a_small_object() {
        let big = "x".repeat(MAX_TO_DEVICE_CONTENT_BYTES);
        let err = parse_recipients(&request(json!({"a": {"1": {"body": big}}})))
            .err()
            .unwrap();
        assert_eq!(limit(err), "to_device_content");
        assert!(parse_recipients(&request(json!({"a": {"1": "text"}}))).is_err());
        let mut req = request(json!({}));
        req.kind = "m.room_key".into();
        assert!(parse_recipients(&req).is_err());
    }

    #[test]
    fn ack_ids_accept_strings_and_numbers() {
        assert_eq!(parse_ids(&[json!("12"), json!(13)]).unwrap(), vec![12, 13]);
        assert!(parse_ids(&[json!("x")]).is_err());
        let err = parse_ids(&vec![json!(1); MAX_TO_DEVICE_ACK_IDS + 1])
            .err()
            .unwrap();
        assert_eq!(limit(err), "to_device_ack");
    }
}
