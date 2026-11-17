//! ユーザーごとのチャンネル・DM のピン留めと並び替え。
//!
//! ピンは本人にだけ見え、並び順もユーザーごとに独立している。書き込みは
//! `user_channel_pin_states` の行をロックして直列にし、`version` で別端末の
//! 更新との競合を検出する。競合したときは最新の `version` と `pins` を
//! 409 `CHANNEL_PINS_CONFLICT` で返し、クライアントが最新の状態の上でやり直せる
//! ようにする。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    middleware,
    response::Response,
    routing::{delete, get, patch},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

use crate::{
    auth::AuthUser, db::get_current_txid, error::AppError, routes::response::ApiResponse,
    state::AppState,
};

/// 1 ユーザーが保持できるピンの上限。アーカイブ中のピンも数える
/// （アンアーカイブや可視状態の変更で 50 件を超えないようにするため）。
pub const MAX_PINNED_CHANNELS: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPinnedChannelResponse {
    pub user_id: String,
    pub channel_id: String,
    pub position: i64,
    #[serde(default)]
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPinsResponse {
    pub pins: Vec<UserPinnedChannelResponse>,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinChannelResponse {
    pub pin: UserPinnedChannelResponse,
    pub version: i64,
    pub pins: Vec<UserPinnedChannelResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnpinChannelResponse {
    pub removed: bool,
    pub version: i64,
    pub pins: Vec<UserPinnedChannelResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReorderChannelPinsResponse {
    pub version: i64,
    pub pins: Vec<UserPinnedChannelResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PinChannelRequest {
    pub channel_id: String,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReorderChannelPinsRequest {
    pub channel_ids: Vec<String>,
    pub expected_version: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UnpinChannelQuery {
    #[serde(default)]
    pub expected_version: Option<i64>,
}

/// ピン留めの対象にできるか。非公開・DM はメンバーだけ、ゲストは公開チャンネルも
/// メンバーであることを求める。`routes/shapes.rs` の where 句と同じ規則。
#[cfg(test)]
pub(crate) fn channel_is_accessible(
    is_private: bool,
    is_dm: bool,
    is_member: bool,
    is_guest: bool,
) -> bool {
    is_member || (!is_private && !is_dm && !is_guest)
}

/// リクエストのチャンネル ID を UUID として検査し、DB と同じ小文字の表記にそろえる。
pub(crate) fn validate_channel_id(raw: &str) -> Result<String, AppError> {
    let parsed = Uuid::parse_str(raw)
        .map_err(|_| AppError::BadRequest(format!("channel_id is not a valid UUID: {raw}")))?;
    Ok(parsed.hyphenated().to_string())
}

fn validate_reorder_ids(channel_ids: &[String]) -> Result<Vec<String>, AppError> {
    // DB を読む前に、上限を超える要求を弾く。並び替えで件数を増やすことはできない。
    if channel_ids.len() > MAX_PINNED_CHANNELS as usize {
        return Err(pin_limit_exceeded());
    }
    let mut seen = HashSet::with_capacity(channel_ids.len());
    let mut validated = Vec::with_capacity(channel_ids.len());
    for raw in channel_ids {
        let id = validate_channel_id(raw)?;
        if !seen.insert(id.clone()) {
            return Err(AppError::BadRequest(
                "channel_ids must not contain duplicates".into(),
            ));
        }
        validated.push(id);
    }
    Ok(validated)
}

/// 次のピン位置。既存の最大位置の次に置く。
pub(crate) fn next_position(max_position: Option<i64>) -> i64 {
    max_position.map_or(0, |position| position.saturating_add(1))
}

/// 並び替えの要求が、いまピン留めされている有効なチャンネルの集合と一致するか。
/// 追加・削除を並び替え API で黙って行わせない。
pub(crate) fn reorder_matches_active_set(
    active: &[PinnedChannelRef],
    requested: &[String],
) -> bool {
    if active.len() != requested.len() {
        return false;
    }
    let active: HashSet<&str> = active.iter().map(|pin| pin.channel_id.as_str()).collect();
    requested.iter().all(|id| active.contains(id.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PinnedChannelRef {
    pub channel_id: String,
    /// アーカイブ中のピン。位置は変えず、そのスロットに残す。
    pub archived: bool,
}

/// チャンネルが見える条件。`{channel}` はチャンネルの別名、`{guest}` はゲストかを
/// 表す bool パラメータ、`{user}` はメンバー判定に使うユーザー ID の式。
/// `routes/shapes.rs` の where 句と同じ規則。
fn channel_visibility_sql(channel: &str, guest: &str, user: &str) -> String {
    format!(
        "((NOT {channel}.is_private AND NOT {channel}.is_dm AND NOT {guest}::boolean) \
         OR EXISTS (SELECT 1 FROM channel_user cu \
                    WHERE cu.channel_id = {channel}.id AND cu.user_id = {user}))"
    )
}

/// 並び替え後の `(channel_id, position)`。アーカイブ中のピンは元のスロットに残し、
/// 有効なピンだけを要求された順に空いたスロットへ当てはめる。アンアーカイブすると
/// 元の位置に戻る。
pub(crate) fn build_pin_order(
    refs: &[PinnedChannelRef],
    requested: &[String],
) -> Vec<(String, i64)> {
    let mut requested = requested.iter();
    refs.iter()
        .enumerate()
        .map(|(position, pin)| {
            let channel_id = if pin.archived {
                pin.channel_id.clone()
            } else {
                requested
                    .next()
                    .cloned()
                    .unwrap_or_else(|| pin.channel_id.clone())
            };
            (channel_id, position as i64)
        })
        .collect()
}

fn pins_conflict(version: i64, pins: Vec<UserPinnedChannelResponse>) -> AppError {
    let mut extra = serde_json::Map::new();
    extra.insert("version".into(), version.into());
    extra.insert(
        "pins".into(),
        serde_json::to_value(pins).expect("pinned channels serialize"),
    );
    AppError::Coded {
        status: StatusCode::CONFLICT,
        code: "CHANNEL_PINS_CONFLICT",
        message: "Channel pins changed on another device; reload and retry".into(),
        extra,
    }
}

fn pin_limit_exceeded() -> AppError {
    let mut extra = serde_json::Map::new();
    extra.insert("limit".into(), MAX_PINNED_CHANNELS.into());
    AppError::Coded {
        status: StatusCode::BAD_REQUEST,
        code: "CHANNEL_PINS_LIMIT_EXCEEDED",
        message: format!("You can pin at most {MAX_PINNED_CHANNELS} channels"),
        extra,
    }
}

fn not_found(channel_id: &str) -> AppError {
    // 非公開チャンネルと DM の存在を推測させないため、権限がない場合も 404 にする。
    AppError::NotFound(format!("Channel not found with id: {channel_id}"))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/channel-pins",
            get(list_channel_pins).post(pin_channel),
        )
        .route("/api/channel-pins/order", patch(reorder_channel_pins))
        .route("/api/channel-pins/{channel_id}", delete(unpin_channel))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

const PIN_JSON: &str = "json_build_object(\
    'user_id', p.user_id, \
    'channel_id', p.channel_id, \
    'position', p.position, \
    'archived', c.archive_at IS NOT NULL, \
    'created_at', p.created_at, \
    'updated_at', p.updated_at\
)::text";

/// いまのユーザーに見えるピンだけを、位置順に返す。アーカイブ中のピンも含む
/// （クライアントが表示だけを止め、アンアーカイブで元の位置に戻す）。
async fn fetch_pins(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    guest: bool,
) -> Result<Vec<UserPinnedChannelResponse>, AppError> {
    let guest = if guest { "true" } else { "false" };
    let visibility = channel_visibility_sql("c", "$2", "p.user_id");
    let sql = format!(
        "SELECT {PIN_JSON} FROM user_pinned_channels p \
         JOIN channels c ON c.id = p.channel_id \
         WHERE p.user_id = $1 AND c.deleted_at IS NULL AND {visibility} \
         ORDER BY p.position, p.channel_id"
    );
    let rows = crate::apps::query(tx, &sql, &[user_id, guest]).await?;
    serde_json::from_value(serde_json::Value::Array(rows))
        .map_err(|err| AppError::Internal(err.into()))
}

async fn fetch_pin(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
) -> Result<Option<UserPinnedChannelResponse>, AppError> {
    let sql = format!(
        "SELECT {PIN_JSON} FROM user_pinned_channels p \
         JOIN channels c ON c.id = p.channel_id \
         WHERE p.user_id = $1 AND p.channel_id = $2"
    );
    let mut rows = crate::apps::query(tx, &sql, &[user_id, channel_id]).await?;
    rows.pop()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| AppError::Internal(err.into()))
}

async fn read_version(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<i64, AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('version', version)::text \
         FROM user_channel_pin_states WHERE user_id = $1",
        &[user_id],
    )
    .await?;
    Ok(rows
        .first()
        .and_then(|row| row["version"].as_i64())
        .unwrap_or(0))
}

/// ユーザーのピン状態の行を作り、`FOR UPDATE` でロックして現在の版を返す。
/// 以降のピンの書き込みは、このロックの中で行う。
pub(crate) async fn lock_pin_state(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<i64, AppError> {
    toasty::sql::statement(
        "INSERT INTO user_channel_pin_states (user_id, version) VALUES ($1, 0) \
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(user_id)
    .exec(tx)
    .await?;
    let row = crate::apps::one(
        tx,
        "SELECT json_build_object('version', version)::text \
         FROM user_channel_pin_states WHERE user_id = $1 FOR UPDATE",
        &[user_id],
    )
    .await?;
    Ok(row["version"].as_i64().unwrap_or(0))
}

async fn bump_version(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<i64, AppError> {
    let row = crate::apps::one(
        tx,
        "UPDATE user_channel_pin_states \
         SET version = version + 1, updated_at = CURRENT_TIMESTAMP \
         WHERE user_id = $1 RETURNING json_build_object('version', version)::text",
        &[user_id],
    )
    .await?;
    Ok(row["version"].as_i64().unwrap_or(0))
}

/// 残ったピンの位置を 0..N-1 に詰める。アーカイブ中のピンも含め、いまの順序を保つ。
async fn compact_positions(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<(), AppError> {
    toasty::sql::statement(
        "WITH ordered AS ( \
            SELECT channel_id, \
                   row_number() OVER (ORDER BY position, channel_id) - 1 AS new_position \
            FROM user_pinned_channels WHERE user_id = $1 \
         ) \
         UPDATE user_pinned_channels p \
         SET position = ordered.new_position, updated_at = CURRENT_TIMESTAMP \
         FROM ordered \
         WHERE p.user_id = $1 AND p.channel_id = ordered.channel_id \
           AND p.position <> ordered.new_position",
    )
    .bind(user_id)
    .exec(tx)
    .await?;
    Ok(())
}

/// チャンネルの閲覧権限を確かめ、ピン留めできるチャンネルならアーカイブ状態を返す。
/// 存在しない・削除済み・閲覧できない場合は 404（存在を推測させない）。
async fn visible_channel(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
    guest: bool,
) -> Result<bool, AppError> {
    let guest = if guest { "true" } else { "false" };
    let visibility = channel_visibility_sql("c", "$3", "$1");
    // `FOR SHARE` で、チャンネルの削除とピンの追加を直列にする。削除が先なら、
    // ロック待ちのあとで `deleted_at` を読み直して 404 になる。
    let sql = format!(
        "SELECT json_build_object('archive_at', c.archive_at)::text \
         FROM channels c \
         WHERE c.id = $2 AND c.deleted_at IS NULL AND {visibility} \
         FOR SHARE OF c"
    );
    let rows = crate::apps::query(tx, &sql, &[user_id, channel_id, guest]).await?;
    let row = rows.first().ok_or_else(|| not_found(channel_id))?;
    Ok(!row["archive_at"].is_null())
}

/// ピン状態の行をロックしたあとに、閲覧権限とアーカイブ状態を新しいスナップショットで
/// 読み直す。`visible_channel` の `FOR SHARE` でチャンネル行は保持しているが、退出・
/// キックはメンバー行の削除とピンの片付けを同じトランザクションで行う。ロック待ちの
/// 間に参加確認のサブクエリが古いスナップショットを見ると、退出後のピンを作れてしまう。
/// ここでは追加のロックを取らず（取得済みのロック順を変えず）に再確認する。
async fn recheck_visible_channel(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
    guest: bool,
) -> Result<bool, AppError> {
    let guest = if guest { "true" } else { "false" };
    let visibility = channel_visibility_sql("c", "$3", "$1");
    let sql = format!(
        "SELECT json_build_object('archive_at', c.archive_at)::text \
         FROM channels c \
         WHERE c.id = $2 AND c.deleted_at IS NULL AND {visibility}"
    );
    let rows = crate::apps::query(tx, &sql, &[user_id, channel_id, guest]).await?;
    let row = rows.first().ok_or_else(|| not_found(channel_id))?;
    Ok(!row["archive_at"].is_null())
}

/// アーカイブ中も含め、ユーザーが保持しているピンの本数。
async fn count_pins(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<i64, AppError> {
    let row = crate::apps::one(
        tx,
        "SELECT json_build_object('count', count(*))::text \
         FROM user_pinned_channels WHERE user_id = $1",
        &[user_id],
    )
    .await?;
    Ok(row["count"].as_i64().unwrap_or(0))
}

async fn max_position(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Option<i64>, AppError> {
    let row = crate::apps::one(
        tx,
        "SELECT json_build_object('max', max(position))::text \
         FROM user_pinned_channels WHERE user_id = $1",
        &[user_id],
    )
    .await?;
    Ok(row["max"].as_i64())
}

/// 削除済み・閲覧できなくなったチャンネルのピンを落とす。並び替えの前に呼ぶ。
async fn prune_inaccessible_pins(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    guest: bool,
) -> Result<(), AppError> {
    let guest = if guest { "true" } else { "false" };
    let visibility = channel_visibility_sql("c", "$2", "p.user_id");
    let sql = format!(
        "DELETE FROM user_pinned_channels p \
         WHERE p.user_id = $1 AND NOT EXISTS ( \
           SELECT 1 FROM channels c \
           WHERE c.id = p.channel_id AND c.deleted_at IS NULL AND {visibility})"
    );
    toasty::sql::statement(&sql)
        .bind(user_id)
        .bind(guest)
        .exec(tx)
        .await?;
    Ok(())
}

async fn pinned_refs(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Vec<PinnedChannelRef>, AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('channel_id', p.channel_id, \
                                  'archived', c.archive_at IS NOT NULL)::text \
         FROM user_pinned_channels p JOIN channels c ON c.id = p.channel_id \
         WHERE p.user_id = $1 \
         ORDER BY p.position, p.channel_id",
        &[user_id],
    )
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PinnedChannelRef {
                channel_id: row["channel_id"]
                    .as_str()
                    .ok_or_else(|| AppError::Internal(anyhow::anyhow!("invalid pin row")))?
                    .to_owned(),
                archived: row["archived"] == true,
            })
        })
        .collect()
}

async fn set_position(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
    position: i64,
) -> Result<(), AppError> {
    toasty::sql::statement(
        "UPDATE user_pinned_channels SET position = $3::integer, updated_at = CURRENT_TIMESTAMP \
         WHERE user_id = $1 AND channel_id = $2",
    )
    .bind(user_id)
    .bind(channel_id)
    .bind(position.to_string())
    .exec(tx)
    .await?;
    Ok(())
}

/// チャンネルから抜ける・キックされる・チャンネルが削除されたときに、その
/// ユーザーのピンと並びを同じトランザクションで片付ける。
///
/// ピンが無くても状態の行は作る。退出と同時に別端末がピンした場合に、
/// 退出後にピンが残って再参加で復活するのを防ぐため、状態の行をロックしてから
/// 削除を確かめる。
pub(crate) async fn remove_user_pin_in_tx(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
) -> Result<bool, AppError> {
    lock_pin_state(tx, user_id).await?;
    let row = crate::apps::one(
        tx,
        "WITH deleted AS ( \
            DELETE FROM user_pinned_channels \
            WHERE user_id = $1 AND channel_id = $2 RETURNING 1 \
         ) SELECT json_build_object('removed', EXISTS (SELECT 1 FROM deleted))::text",
        &[user_id, channel_id],
    )
    .await?;
    let removed = row["removed"] == true;
    if removed {
        compact_positions(tx, user_id).await?;
        bump_version(tx, user_id).await?;
    }
    Ok(removed)
}

/// チャンネルを削除したときに、全ユーザーのそのチャンネルのピンを片付ける。
/// 対象ユーザーを `user_id` 順に固定して状態の行をまとめてロックし、位置の詰め直しと
/// 版の更新も集合単位の SQL で行う（ユーザー数に比例した問い合わせをしない）。
pub(crate) async fn remove_channel_pins_in_tx(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
) -> Result<(), AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('user_id', user_id)::text \
         FROM user_pinned_channels WHERE channel_id = $1 \
         GROUP BY user_id ORDER BY user_id",
        &[channel_id],
    )
    .await?;
    let user_ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row["user_id"].as_str().map(str::to_owned))
        .collect();
    if user_ids.is_empty() {
        return Ok(());
    }

    // 状態の行を作ってから、`user_id` 順にロックする。既存の 1 ユーザー向けの
    // `remove_user_pin_in_tx` と同じく、退出と同時のピン留めも直列になる。
    // INSERT 自体も `user_id` 順に固定する。並行するチャンネル削除どうしが
    // 逆順で PRIMARY KEY の行を挿入し合ってデッドロックするのを防ぐ。
    toasty::sql::statement(
        "INSERT INTO user_channel_pin_states (user_id, version) \
         SELECT user_id, 0 FROM user_pinned_channels WHERE channel_id = $1 \
         ORDER BY user_id \
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(channel_id)
    .exec(tx)
    .await?;
    let array = format!("{{{}}}", user_ids.join(","));
    crate::apps::query(
        tx,
        "SELECT json_build_object('user_id', user_id)::text \
         FROM user_channel_pin_states WHERE user_id = ANY($1::text[]) \
         ORDER BY user_id FOR UPDATE",
        &[&array],
    )
    .await?;

    toasty::sql::statement("DELETE FROM user_pinned_channels WHERE channel_id = $1")
        .bind(channel_id)
        .exec(tx)
        .await?;
    toasty::sql::statement(
        "WITH ordered AS ( \
            SELECT user_id, channel_id, \
                   row_number() OVER (PARTITION BY user_id ORDER BY position, channel_id) - 1 \
                       AS new_position \
            FROM user_pinned_channels WHERE user_id = ANY($1::text[]) \
         ) \
         UPDATE user_pinned_channels p \
         SET position = ordered.new_position, updated_at = CURRENT_TIMESTAMP \
         FROM ordered \
         WHERE p.user_id = ordered.user_id AND p.channel_id = ordered.channel_id \
           AND p.position <> ordered.new_position",
    )
    .bind(&array)
    .exec(tx)
    .await?;
    toasty::sql::statement(
        "UPDATE user_channel_pin_states \
         SET version = version + 1, updated_at = CURRENT_TIMESTAMP \
         WHERE user_id = ANY($1::text[])",
    )
    .bind(&array)
    .exec(tx)
    .await?;
    Ok(())
}

async fn get_guest(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<bool, AppError> {
    crate::apps::events::guest(tx, user_id).await
}

async fn list_channel_pins(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<ChannelPinsResponse>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 閲覧権限・ピン・版を別々の SELECT で読むので、同じスナップショットにそろえる。
    // そろえないと、間に別端末の書き込みがコミットしたとき、古い版に新しいピンを
    // 組み合わせて返し、クライアントの次の書き込みが不要に競合する。
    toasty::sql::statement("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .exec(&mut tx)
        .await?;
    let guest = get_guest(&mut tx, &auth_user.id).await?;
    let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
    let version = read_version(&mut tx, &auth_user.id).await?;
    tx.commit().await?;
    Ok(Json(ChannelPinsResponse { pins, version }))
}

async fn pin_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<PinChannelRequest>,
) -> Result<Json<ApiResponse<PinChannelResponse>>, AppError> {
    let channel_id = validate_channel_id(&payload.channel_id)?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let guest = get_guest(&mut tx, &auth_user.id).await?;
    let archived = visible_channel(&mut tx, &auth_user.id, &channel_id, guest).await?;
    if archived {
        return Err(AppError::BadRequest(
            "Cannot pin an archived channel".into(),
        ));
    }

    let version = lock_pin_state(&mut tx, &auth_user.id).await?;
    // 退出・キックはメンバー行の削除後に同じ状態行をロックする。最初の閲覧確認が
    // ロック待ちの間に古いスナップショットを見ていても、ここで参加状態を読み直す。
    let archived = recheck_visible_channel(&mut tx, &auth_user.id, &channel_id, guest).await?;
    if archived {
        let _ = tx.rollback().await;
        return Err(AppError::BadRequest(
            "Cannot pin an archived channel".into(),
        ));
    }
    // 閲覧できなくなったピンを先に落とし、残っている件数で上限を判定する。
    prune_inaccessible_pins(&mut tx, &auth_user.id, guest).await?;

    if let Some(expected) = payload.expected_version
        && expected != version
    {
        let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
        let _ = tx.rollback().await;
        return Err(pins_conflict(version, pins));
    }

    if let Some(pin) = fetch_pin(&mut tx, &auth_user.id, &channel_id).await? {
        // 再送。並びは変えず、いまの版と一覧を返す。
        let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
        let _ = tx.rollback().await;
        return Ok(Json(ApiResponse::new(
            PinChannelResponse { pin, version, pins },
            None,
        )));
    }

    // アーカイブ中のピンも数える。アンアーカイブで 50 件を超えないようにするため。
    if count_pins(&mut tx, &auth_user.id).await? >= MAX_PINNED_CHANNELS {
        let _ = tx.rollback().await;
        return Err(pin_limit_exceeded());
    }

    let position = next_position(max_position(&mut tx, &auth_user.id).await?);
    crate::apps::query(
        &mut tx,
        "INSERT INTO user_pinned_channels (user_id, channel_id, position) \
         VALUES ($1, $2, $3::integer) \
         RETURNING json_build_object('inserted', true)::text",
        &[&auth_user.id, &channel_id, &position.to_string()],
    )
    .await?;
    let version = bump_version(&mut tx, &auth_user.id).await?;
    let pin = fetch_pin(&mut tx, &auth_user.id, &channel_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("pinned channel disappeared")))?;
    let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        PinChannelResponse { pin, version, pins },
        txid,
    )))
}

async fn unpin_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<UnpinChannelQuery>,
) -> Result<Json<ApiResponse<UnpinChannelResponse>>, AppError> {
    let channel_id = validate_channel_id(&channel_id)?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let guest = get_guest(&mut tx, &auth_user.id).await?;
    let version = lock_pin_state(&mut tx, &auth_user.id).await?;

    if let Some(expected) = query.expected_version
        && expected != version
    {
        let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
        let _ = tx.rollback().await;
        return Err(pins_conflict(version, pins));
    }

    let removed = remove_user_pin_in_tx(&mut tx, &auth_user.id, &channel_id).await?;
    let version = if removed {
        read_version(&mut tx, &auth_user.id).await?
    } else {
        version
    };
    let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
    let txid = if removed {
        Some(get_current_txid(&mut tx).await?)
    } else {
        None
    };
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        UnpinChannelResponse {
            removed,
            version,
            pins,
        },
        txid,
    )))
}

async fn reorder_channel_pins(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<ReorderChannelPinsRequest>,
) -> Result<Json<ApiResponse<ReorderChannelPinsResponse>>, AppError> {
    let requested = validate_reorder_ids(&payload.channel_ids)?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let guest = get_guest(&mut tx, &auth_user.id).await?;

    // 追加・削除を並び替えで黙って行わない。要求されたチャンネルは、いま
    // ピン留めされている有効なチャンネルでなければならない。
    for id in &requested {
        // アーカイブ中かどうかは集合の比較で判定する。ここでは閲覧できることだけ確かめる。
        if let Err(error) = visible_channel(&mut tx, &auth_user.id, id, guest).await {
            // 退出・削除で見えなくなったチャンネルを含む古い並びは、404 ではなく
            // 版の競合として返す。クライアントは最新の一覧を採用して再試行できる。
            // ロック順を state -> channel に変えないよう、閲覧確認の後で状態行を取る。
            let version = lock_pin_state(&mut tx, &auth_user.id).await?;
            if payload.expected_version != version {
                let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
                let _ = tx.rollback().await;
                return Err(pins_conflict(version, pins));
            }
            let _ = tx.rollback().await;
            return Err(error);
        }
    }

    let version = lock_pin_state(&mut tx, &auth_user.id).await?;
    if payload.expected_version != version {
        let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
        let _ = tx.rollback().await;
        return Err(pins_conflict(version, pins));
    }

    prune_inaccessible_pins(&mut tx, &auth_user.id, guest).await?;
    let refs = pinned_refs(&mut tx, &auth_user.id).await?;
    let active: Vec<PinnedChannelRef> = refs.iter().filter(|pin| !pin.archived).cloned().collect();
    if !reorder_matches_active_set(&active, &requested) {
        let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
        let _ = tx.rollback().await;
        return Err(pins_conflict(version, pins));
    }

    for (channel_id, position) in build_pin_order(&refs, &requested) {
        set_position(&mut tx, &auth_user.id, &channel_id, position).await?;
    }
    let version = bump_version(&mut tx, &auth_user.id).await?;
    let pins = fetch_pins(&mut tx, &auth_user.id, guest).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ReorderChannelPinsResponse { version, pins },
        txid,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_channel_ids_as_uuids() {
        let raw = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
        assert_eq!(validate_channel_id(raw).unwrap(), raw);
        assert_eq!(
            validate_channel_id("3F2504E0-4F89-41D3-9A0C-0305E82C3301").unwrap(),
            raw
        );
        assert_eq!(
            validate_channel_id("3F2504E04F8941D39A0C0305E82C3301").unwrap(),
            raw
        );
        let err = validate_channel_id("not-a-uuid").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn reorder_rejects_duplicate_ids() {
        let id = "3f2504e0-4f89-41d3-9a0c-0305e82c3301".to_string();
        let err = validate_reorder_ids(&[id.clone(), id]).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(message) if message.contains("duplicates")));
    }

    #[test]
    fn reorder_allows_an_empty_order() {
        assert!(validate_reorder_ids(&[]).unwrap().is_empty());
    }

    #[test]
    fn reorder_rejects_more_ids_than_the_limit_before_querying() {
        let ids: Vec<String> = (0..=MAX_PINNED_CHANNELS)
            .map(|_| Uuid::new_v4().to_string())
            .collect();
        let err = validate_reorder_ids(&ids).unwrap_err();
        match err {
            AppError::Coded { status, code, .. } => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(code, "CHANNEL_PINS_LIMIT_EXCEEDED");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn access_rules_hide_private_and_dm_from_non_members() {
        // 公開チャンネルは、ゲストでなければメンバーでなくても見える。
        assert!(channel_is_accessible(false, false, false, false));
        assert!(!channel_is_accessible(false, false, false, true));
        // 非公開・DM はメンバーだけ。
        assert!(!channel_is_accessible(true, false, false, false));
        assert!(!channel_is_accessible(false, true, false, false));
        assert!(channel_is_accessible(true, false, true, false));
        assert!(channel_is_accessible(false, true, true, true));
    }

    #[test]
    fn next_position_appends_after_the_current_maximum() {
        assert_eq!(next_position(None), 0);
        assert_eq!(next_position(Some(0)), 1);
        assert_eq!(next_position(Some(49)), 50);
    }

    #[test]
    fn reorder_requires_the_exact_active_set() {
        let active = vec![
            PinnedChannelRef {
                channel_id: "a".into(),
                archived: false,
            },
            PinnedChannelRef {
                channel_id: "b".into(),
                archived: false,
            },
        ];
        assert!(reorder_matches_active_set(
            &active,
            &["b".into(), "a".into()]
        ));
        // 追加・削除・重複は一致しない。
        assert!(!reorder_matches_active_set(&active, &["a".into()]));
        assert!(!reorder_matches_active_set(
            &active,
            &["a".into(), "b".into(), "c".into()]
        ));
        assert!(reorder_matches_active_set(&[], &[]));
    }

    #[test]
    fn order_fills_active_slots_and_keeps_archived_pins_in_place() {
        let refs = vec![
            PinnedChannelRef {
                channel_id: "a".into(),
                archived: false,
            },
            PinnedChannelRef {
                channel_id: "old".into(),
                archived: true,
            },
            PinnedChannelRef {
                channel_id: "c".into(),
                archived: false,
            },
        ];
        assert_eq!(
            build_pin_order(&refs, &["c".into(), "a".into()]),
            vec![
                ("c".to_string(), 0),
                ("old".to_string(), 1),
                ("a".to_string(), 2),
            ]
        );
    }

    #[test]
    fn conflict_carries_version_and_latest_pins() {
        let pins = vec![UserPinnedChannelResponse {
            user_id: "u1".into(),
            channel_id: "c1".into(),
            position: 0,
            archived: false,
            created_at: "2026-11-16T00:00:00.000".into(),
            updated_at: "2026-11-16T00:00:00.000".into(),
        }];
        let error = pins_conflict(7, pins.clone());
        match error {
            AppError::Coded {
                status,
                code,
                ref extra,
                ..
            } => {
                assert_eq!(status, StatusCode::CONFLICT);
                assert_eq!(code, "CHANNEL_PINS_CONFLICT");
                assert_eq!(extra["version"], 7);
                assert_eq!(
                    extra["pins"],
                    serde_json::to_value(&pins).expect("pins serialize")
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn limit_error_is_coded_and_names_the_limit() {
        let error = pin_limit_exceeded();
        match error {
            AppError::Coded {
                status,
                code,
                ref extra,
                ..
            } => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(code, "CHANNEL_PINS_LIMIT_EXCEEDED");
                assert_eq!(extra["limit"], MAX_PINNED_CHANNELS);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn conflict_response_body_has_the_documented_fields() {
        let pin = UserPinnedChannelResponse {
            user_id: "u1".into(),
            channel_id: "c1".into(),
            position: 0,
            archived: false,
            created_at: "2026-11-16T00:00:00.000".into(),
            updated_at: "2026-11-16T00:00:00.000".into(),
        };
        let response = pins_conflict(3, vec![pin.clone()]).into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "CHANNEL_PINS_CONFLICT");
        assert_eq!(body["version"], 3);
        assert_eq!(body["pins"], serde_json::to_value([pin]).unwrap());
    }

    use axum::response::IntoResponse;

    #[test]
    fn serialize_list_response() {
        let value = serde_json::to_value(ChannelPinsResponse {
            pins: Vec::new(),
            version: 0,
        })
        .unwrap();
        assert_eq!(value, serde_json::json!({"pins": [], "version": 0}));
    }
}
