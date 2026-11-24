use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelNote, ChannelUser},
    routes::response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct NoteResponse {
    pub id: String,
    pub channel_id: String,
    pub title: String,
    pub content: String,
    /// 表示順。有効ノート一覧のページ継続カーソルにも使う。
    pub position: i32,
    pub version: i32,
    pub created_user_id: String,
    pub created_at: String,
    pub updated_at: String,
    /// 削除済み一覧でのみ設定する。次ページのカーソルにも使う。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

impl From<ChannelNote> for NoteResponse {
    fn from(note: ChannelNote) -> Self {
        Self {
            deleted_at: None,
            id: note.id,
            channel_id: note.channel_id,
            title: note.title,
            content: note.content,
            position: note.position,
            version: note.version,
            created_user_id: note.created_user_id,
            created_at: note.created_at.to_string(),
            updated_at: note.updated_at.to_string(),
        }
    }
}

/// 有効ノート一覧の項目。本文は含まず、取得 API (`GET /notes/{id}`) で別途読む。
/// `order_token` はチャンネルの有効ノート全体の `(position, id)` に対するハッシュで、
/// 追加・削除・復元・並べ替えのいずれでも変化する。ページ継続カーソルに使う。
#[derive(Debug, Serialize)]
pub struct NoteSummaryResponse {
    pub id: String,
    pub channel_id: String,
    pub title: String,
    /// 表示順。有効ノート一覧のページ継続カーソルにも使う。
    pub position: i32,
    pub version: i32,
    pub created_user_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub order_token: String,
}

/// 削除済みノート一覧の項目。本文は含まず、`GET /notes/deleted/{note_id}` で別途読む。
#[derive(Debug, Serialize)]
pub struct DeletedNoteSummaryResponse {
    pub id: String,
    pub channel_id: String,
    pub title: String,
    /// 表示順。有効ノート一覧と同じ値。
    pub position: i32,
    pub version: i32,
    pub created_user_id: String,
    pub created_at: String,
    pub updated_at: String,
    /// 次ページのカーソルに使う。UTC の `YYYY-MM-DDTHH:MM:SS.mmm` 固定形式。
    pub deleted_at: String,
}

#[derive(Deserialize)]
pub struct NoteInput {
    title: String,
    content: String,
}

#[derive(Deserialize)]
pub struct UpdateNoteInput {
    title: String,
    content: String,
    version: i32,
}

#[derive(Deserialize)]
pub struct DeleteNoteInput {
    version: i32,
}

#[derive(Deserialize)]
pub struct ReorderNotesInput {
    expected_note_ids: Vec<String>,
    note_ids: Vec<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{channel_id}/notes", get(list).post(create))
        .route(
            "/api/channels/{channel_id}/notes/deleted",
            get(list_deleted),
        )
        .route(
            "/api/channels/{channel_id}/notes/deleted/{note_id}",
            get(get_deleted),
        )
        .route(
            "/api/channels/{channel_id}/notes/order",
            axum::routing::patch(reorder),
        )
        .route(
            "/api/channels/{channel_id}/notes/{note_id}",
            get(show).patch(update).delete(remove),
        )
        .route(
            "/api/channels/{channel_id}/notes/{note_id}/restore",
            axum::routing::post(restore),
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

/// Delete only expired tombstones, in bounded batches shared safely by workers.
pub async fn purge_expired_notes(db: &toasty::Db) -> Result<u64, AppError> {
    let db = db.clone();
    let mut cleanup_db = db.clone();
    let mut tx = cleanup_db.transaction().await?;
    let cursor_rows = toasty::sql::query(
        "SELECT last_channel_id FROM channel_notes_cleanup_cursor WHERE id = true FOR UPDATE",
    )
    .exec(&mut tx)
    .await?;
    let mut last_channel = match cursor_rows.into_iter().next() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.into_iter().next() {
            Some(toasty::stmt::Value::String(value)) => Some(value),
            Some(toasty::stmt::Value::Null) => None,
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected cleanup cursor"
                )));
            }
        },
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Missing cleanup cursor"
            )));
        }
    };
    let mut unlocked_channels = Vec::with_capacity(100);
    let mut seen_channels = std::collections::HashSet::new();
    let mut scanned = 0usize;
    let mut wrapped = false;
    while unlocked_channels.len() < 100 && scanned < 500 {
        // Page over the channels primary key rather than grouping every
        // expired note. This bounds each lookup to 100 channels and uses the
        // channel_notes expiry index for an existence probe per channel.
        let page_rows = toasty::sql::query(
            "WITH page AS MATERIALIZED (\
                 SELECT id FROM channels WHERE id > $1 ORDER BY id LIMIT $2\
             ) \
             SELECT page.id, expired.id IS NOT NULL AS eligible FROM page \
             LEFT JOIN LATERAL (\
                 SELECT id FROM channel_notes \
                 WHERE channel_id = page.id \
                   AND deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
                 ORDER BY deleted_at, id LIMIT 1\
             ) expired ON true ORDER BY page.id",
        )
        .bind(last_channel.clone().unwrap_or_default())
        .bind((100 - unlocked_channels.len()).min(500 - scanned) as i32)
        .exec(&mut tx)
        .await?;
        let mut candidates = Vec::with_capacity(page_rows.len());
        let mut page_last = None;
        for value in &page_rows {
            if let toasty::stmt::Value::Record(record) = value {
                let mut fields = record.fields.iter();
                if let (Some(toasty::stmt::Value::String(id)), Some(eligible)) =
                    (fields.next(), fields.next())
                {
                    page_last = Some(id.clone());
                    if matches!(eligible, toasty::stmt::Value::Bool(true)) {
                        candidates.push(id.clone());
                    }
                }
            }
        }
        let Some(page_last) = page_last else {
            if wrapped || last_channel.is_none() {
                break;
            }
            toasty::sql::statement(
                "UPDATE channel_notes_cleanup_cursor SET last_channel_id = NULL WHERE id = true",
            )
            .exec(&mut tx)
            .await?;
            last_channel = None;
            wrapped = true;
            continue;
        };
        toasty::sql::statement(
            "UPDATE channel_notes_cleanup_cursor SET last_channel_id = $1 WHERE id = true",
        )
        .bind(page_last.clone())
        .exec(&mut tx)
        .await?;
        last_channel = Some(page_last);

        let selected = serde_json::to_string(&candidates)
            .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
        let lock_rows = toasty::sql::query(
            "SELECT channel_id, pg_try_advisory_xact_lock(\
                 hashtextextended('channel-notes:' || channel_id, 0)\
             ) AS locked \
             FROM jsonb_array_elements_text($1::jsonb) AS channel_id",
        )
        .bind(selected)
        .exec(&mut tx)
        .await?;
        for row in lock_rows {
            if let toasty::stmt::Value::Record(record) = row {
                let mut fields = record.fields.into_iter();
                if let (
                    Some(toasty::stmt::Value::String(channel_id)),
                    Some(toasty::stmt::Value::Bool(true)),
                ) = (fields.next(), fields.next())
                    && seen_channels.insert(channel_id.clone())
                {
                    unlocked_channels.push(channel_id);
                }
            }
        }
        scanned += page_rows.len();
    }
    // Drop candidates that became empty while their advisory locks were being
    // taken. This keeps the next loop from issuing one no-op DELETE per such
    // channel (for example, 999 empty candidates and one non-empty channel).
    let available_rows = toasty::sql::query(
        "SELECT c.channel_id FROM jsonb_array_elements_text($1::jsonb) AS c(channel_id) \
         WHERE EXISTS (SELECT 1 FROM channel_notes n \
           WHERE n.channel_id=c.channel_id \
           AND n.deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours')",
    )
    .bind(serde_json::json!(unlocked_channels).to_string())
    .exec(&mut tx)
    .await?;
    let mut available = std::collections::HashSet::new();
    for row in available_rows {
        if let toasty::stmt::Value::Record(record) = row
            && let Some(toasty::stmt::Value::String(channel_id)) = record.fields.into_iter().next()
        {
            available.insert(channel_id);
        }
    }
    let mut count = 0u64;
    let mut active_channels: Vec<String> = unlocked_channels
        .into_iter()
        .filter(|channel_id| available.contains(channel_id))
        .collect();
    while count < 100 && !active_channels.is_empty() {
        let before_pass = count;
        let pass_remaining = (100 - count) as usize;
        // Share the bounded batch across every channel active at the start of
        // this pass. Recomputing the share per item would give the last
        // channels a much larger slice and leave older rows in the first
        // channels waiting for a later pass.
        let per_channel = pass_remaining.div_ceil(active_channels.len());
        let mut next_active = Vec::new();
        for channel_id in &active_channels {
            let limit = per_channel.min((100 - count) as usize);
            if limit == 0 {
                break;
            }
            // 先頭の limit 件だけを候補として読み、その中の未ロック行を削除する。
            // LIMIT が SKIP LOCKED の後にあると、ロック済み行を返却件数に数えないため、削除が
            // 0 件でも期限切れの全行を走査し、この間の API 書き込みを advisory lock で待たせる。
            // スキップした行は次の回転で再び候補になる。
            let deleted = toasty::sql::statement(
                "WITH candidates AS MATERIALIZED (
                SELECT id FROM channel_notes
                WHERE channel_id = $1
                  AND deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours'
                ORDER BY deleted_at, id
                LIMIT $2
            ), locked AS (
                SELECT c.id FROM candidates c JOIN channel_notes n ON n.id = c.id
                FOR UPDATE OF n SKIP LOCKED
            ) DELETE FROM channel_notes WHERE id IN (SELECT id FROM locked)",
            )
            .bind(channel_id.as_str())
            .bind(limit as i32)
            .exec(&mut tx)
            .await?;
            count += deleted;
            // A channel that could not fill its share is depleted. Do not count
            // it in the denominator of the next pass.
            if deleted >= per_channel as u64 {
                next_active.push(channel_id.clone());
            }
        }
        if count == before_pass {
            break;
        }
        active_channels = next_active;
    }
    tx.commit().await?;
    Ok(count)
}

pub async fn cleanup_worker(db: toasty::Db) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await;
        if let Err(error) = purge_expired_notes(&db).await {
            tracing::warn!(%error, "Expired channel note cleanup failed; will retry");
        }
    }
}

async fn authorize(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel_id: &str,
    write: bool,
) -> Result<Channel, AppError> {
    Uuid::parse_str(channel_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    // AuthUser's active_user check runs in a separate transaction. Recheck and
    // hold the user row through note retrieval; guest_kind updates (including
    // deactivation) update this row before their triggers touch channel rows.
    let active_user =
        toasty::sql::query("SELECT id FROM users WHERE id = $1 AND NOT is_deactivated FOR SHARE")
            .bind(actor)
            .exec(&mut *tx)
            .await?;
    if active_user.is_empty() {
        return Err(AppError::Forbidden("Account is deactivated".into()));
    }
    let guest = crate::apps::events::guest(tx, actor).await?;

    // Lock order: users, channels, then channel_user. Keep the channel lock
    // shared for note writes: the database trigger for direct note updates
    // locks the note row before taking the same shared channel lock. A stronger
    // lock here would deadlock with that direct-SQL path.
    if write {
        toasty::sql::query("SELECT id FROM channels WHERE id = $1 FOR SHARE")
            .bind(channel_id)
            .exec(&mut *tx)
            .await?;
        // Serialize authorization with concurrent removal of this membership.
        toasty::sql::query(
            "SELECT user_id FROM channel_user WHERE channel_id = $1 AND user_id = $2 FOR UPDATE",
        )
        .bind(channel_id)
        .bind(actor)
        .exec(&mut *tx)
        .await?;
    } else {
        // Hold a shared channel lock through note retrieval so privacy and
        // membership changes which lock this row first serialize with reads.
        toasty::sql::query("SELECT id FROM channels WHERE id = $1 FOR SHARE")
            .bind(channel_id)
            .exec(&mut *tx)
            .await?;
        // App, webhook, and Slack cleanup paths can delete channel_user rows
        // without locking channels. Lock this actor's membership too, so those
        // removals cannot commit between authorization and note retrieval.
        toasty::sql::query(
            "SELECT user_id FROM channel_user WHERE channel_id = $1 AND user_id = $2 FOR SHARE",
        )
        .bind(channel_id)
        .bind(actor)
        .exec(&mut *tx)
        .await?;
    }
    let channel = Channel::filter(Channel::fields().id().eq(channel_id))
        .first()
        .exec(&mut *tx)
        .await?
        .filter(|channel| channel.deleted_at.is_none())
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id)
            .and(ChannelUser::fields().user_id().eq(actor)),
    )
    .first()
    .exec(&mut *tx)
    .await?
    .is_some();

    if (channel.is_private || channel.is_dm || guest) && !member {
        return Err(AppError::NotFound("Channel not found".into()));
    }
    // Notes are stored as plaintext and cannot yet honor E2EE guarantees.
    if channel.encrypted {
        return Err(AppError::BadRequest(
            "Notes are unavailable in encrypted channels".into(),
        ));
    }
    if write {
        if !member {
            return Err(AppError::Forbidden(
                "Join the channel to manage notes".into(),
            ));
        }
        if channel.archive_at.is_some() {
            return Err(AppError::BadRequest(
                "Cannot modify notes in an archived channel".into(),
            ));
        }
        // Serialize API note mutations within this channel before reading note
        // order or locking note rows. Direct SQL writers that coordinate with
        // this API must take the same lock before their note statement.
        toasty::sql::query(
            "SELECT '{}'::text FROM pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
        )
        .bind(channel_id)
        .exec(&mut *tx)
        .await?;
    }
    Ok(channel)
}

/// A channel's active notes are returned and reordered in one request, so the
/// API keeps their count bounded. Direct SQL writers that follow the note-lock
/// contract should apply the same limit.
pub const MAX_ACTIVE_NOTES_PER_CHANNEL: i64 = 500;

async fn ensure_note_capacity(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
) -> Result<(), AppError> {
    let rows = toasty::sql::query(
        "SELECT count(*)::int AS count FROM channel_notes \
         WHERE channel_id = $1 AND deleted_at IS NULL",
    )
    .bind(channel_id)
    .exec(tx)
    .await?;
    let count = match rows.first() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.first() {
            Some(toasty::stmt::Value::I32(value)) => *value as i64,
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected channel note count"
                )));
            }
        },
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Missing channel note count"
            )));
        }
    };
    if count >= MAX_ACTIVE_NOTES_PER_CHANNEL {
        return Err(AppError::Conflict(format!(
            "A channel can have at most {MAX_ACTIVE_NOTES_PER_CHANNEL} active notes"
        )));
    }
    Ok(())
}

/// 有効ノートの `position` を `(position, id)` 順の連番 `0, 1, 2, ...` に詰め直す。
/// ノートロック契約に従う直接 SQL は任意の `position` を書けるため、
/// `MAX(position)` が `i32::MAX` に達すると作成 (`+ 1`) や復元の位置ずらしが
/// integer オーバーフローで 500 になる。詰め直しは相対順序を変えないため、
/// 並び替え結果と一覧の表示順は保たれたまま、以後の挿入・ずらしの余地を作る。
/// 戻り値は詰め直した行数 (= 新しい末尾位置)。
async fn compact_active_note_positions(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    prioritize_note_id: Option<&str>,
) -> Result<i32, AppError> {
    let rows = match prioritize_note_id {
        Some(restored_id) => {
            toasty::sql::query(
                "WITH compacted AS (\
                     SELECT id, (ROW_NUMBER() OVER (ORDER BY position, (id <> $2), id) - 1)::int AS position \
                     FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL\
                 ) \
                 UPDATE channel_notes SET position = compacted.position \
                 FROM compacted WHERE channel_notes.id = compacted.id \
                 RETURNING compacted.position",
            )
            .bind(channel_id)
            .bind(restored_id)
            .exec(tx)
            .await?
        }
        None => {
            toasty::sql::query(
                "WITH compacted AS (\
                     SELECT id, (ROW_NUMBER() OVER (ORDER BY position, id) - 1)::int AS position \
                     FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL\
                 ) \
                 UPDATE channel_notes SET position = compacted.position \
                 FROM compacted WHERE channel_notes.id = compacted.id \
                 RETURNING compacted.position",
            )
            .bind(channel_id)
            .exec(tx)
            .await?
        }
    };
    // 行数 = 連番の個数 = 新しい末尾位置 + 1。上限 500 件のため i32 に収まる。
    // position 列は integer だが、::int を付けない window 関数の結果は bigint で
    // 返り得るため、両方の表現を受け付ける。
    let count = rows
        .into_iter()
        .map(|row| match row {
            toasty::stmt::Value::Record(record) => match record.fields.into_iter().next() {
                Some(toasty::stmt::Value::I32(position)) => Ok(position),
                Some(toasty::stmt::Value::I64(position)) => Ok(position as i32),
                other => Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected compacted channel note position: {other:?}"
                ))),
            },
            other => Err(AppError::Internal(anyhow::anyhow!(
                "Unexpected compacted channel note row: {other:?}"
            ))),
        })
        .collect::<Result<Vec<i32>, _>>()?
        .len() as i32;
    Ok(count)
}

fn validate(input: &NoteInput) -> Result<(), AppError> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(AppError::BadRequest(
            "Note title must be between 1 and 200 characters".into(),
        ));
    }
    if input.title.contains('\0') || input.content.contains('\0') {
        return Err(AppError::BadRequest(
            "Note title and content must not contain NUL".into(),
        ));
    }
    if input.content.chars().count() > 100_000 {
        return Err(AppError::BadRequest("Note content is too long".into()));
    }
    Ok(())
}

fn note_path(channel_id: &str, note_id: &str) -> Result<(), AppError> {
    Uuid::parse_str(channel_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    // 一覧は保存済みの id をそのまま返す。作成 API は UUID を保存するが、ノートロック契約に
    // 従う直接 SQL は任意の text id を書き得るため、項目 API も同じ形式を受け付ける。
    if note_id.is_empty() {
        return Err(AppError::BadRequest("Invalid note ID".into()));
    }
    Ok(())
}

#[derive(Deserialize)]
struct ActiveNotesQuery {
    limit: Option<i64>,
    /// `"<order_token> <position> <id>"`。前ページの最後の項目から継続する。
    after: Option<String>,
}

/// 有効ノート一覧のページ継続カーソルを検証する。戻り値は
/// `(前ページ取得時の order_token, position, id)`。トークンは最初のページでは不要で、
/// 指定された場合のみ並び替え・追加・削除・復元の競合検出に使う。
///
/// 直接 SQL はノートロック契約に従えば任意の text id を書けるため、区切りの空白が
/// 存在する限り `id` は空文字 (`""`) を許す (`after = "<token> <position> "`)。
/// 区切りそのものの欠落、空トークン、空 position、不正な position は 400 にする。
fn parse_active_after(after: Option<&str>) -> Result<(Option<String>, String, String), AppError> {
    match after {
        None | Some("") => Ok((None, String::new(), String::new())),
        Some(cursor) => {
            let Some((token, rest)) = cursor.split_once(' ') else {
                return Err(AppError::BadRequest("Invalid note cursor".into()));
            };
            let Some((position, id)) = rest.split_once(' ') else {
                return Err(AppError::BadRequest("Invalid note cursor".into()));
            };
            if token.is_empty() || position.is_empty() || position.parse::<i32>().is_err() {
                return Err(AppError::BadRequest("Invalid note cursor".into()));
            }
            Ok((
                Some(token.to_string()),
                position.to_string(),
                id.to_string(),
            ))
        }
    }
}

async fn list(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<ActiveNotesQuery>,
) -> Result<Json<ApiResponse<Vec<NoteSummaryResponse>>>, AppError> {
    // 一覧は本文を返さない (取得 API で本文を別途読む)。それでも 1 回の応答で全件は
    // 返さず、ページは SQL で制限し、(position, id) のカーソルで表示順を保ったまま
    // 続きを取得できる。order_token は同じ文で有効ノート全体から計算し、継続ページで
    // 前ページ取得時の並び・構成と比較する (Read COMMITTED では別文の比較になり得る
    // 競合を、1 文のスナップショットに収める)。
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as i32;
    let (cursor_token, after_position, after_id) = parse_active_after(query.after.as_deref())?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    let rows = toasty::sql::query(
        "WITH ordering AS (\
             SELECT md5(COALESCE(string_agg(position::text || ':' || encode(convert_to(id, 'UTF8'), 'hex'), ',' ORDER BY position, id), '')) AS order_token \
             FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL\
         ), page AS (\
             SELECT id, channel_id, title, position, version, created_user_id, \
                    to_char(created_at, 'YYYY-MM-DD') || 'T' || to_char(created_at, 'HH24:MI:SS.MS') AS created_at, \
                    to_char(updated_at, 'YYYY-MM-DD') || 'T' || to_char(updated_at, 'HH24:MI:SS.MS') AS updated_at \
             FROM channel_notes \
             WHERE channel_id = $1 AND deleted_at IS NULL \
               AND (CASE WHEN $2 = '' THEN true \
                         ELSE (position, id) > ($2::integer, $3) END) \
             ORDER BY position, id LIMIT $4\
         ) \
         SELECT page.id, page.channel_id, page.title, page.position, page.version, \
                page.created_user_id, page.created_at, page.updated_at, ordering.order_token \
         FROM ordering LEFT JOIN page ON true \
         ORDER BY page.position, page.id",
    )
    .bind(channel_id.as_str())
    .bind(after_position.as_str())
    .bind(after_id.as_str())
    .bind(limit)
    .exec(&mut tx)
    .await?;
    // ordering は集約 1 行なので、LEFT JOIN は必ず 1 行以上を返す。ページが空の
    // 場合は note 列だけ NULL の行が 1 行来るため、どのページでも現在の
    // order_token を比較できる。
    let mut result = Vec::new();
    let mut current_token = String::new();
    for row in rows {
        let toasty::stmt::Value::Record(record) = row else {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Unexpected channel note row"
            )));
        };
        let mut fields = record.fields.into_iter();
        let id = fields.next();
        let channel_id = fields.next();
        let title = fields.next();
        let position = fields.next();
        let version = fields.next();
        let created_user_id = fields.next();
        let created_at = fields.next();
        let updated_at = fields.next();
        let order_token = fields.next();
        match order_token {
            Some(toasty::stmt::Value::String(token)) => current_token = token,
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected channel note order token"
                )));
            }
        }
        let (
            Some(toasty::stmt::Value::String(id)),
            Some(toasty::stmt::Value::String(channel_id)),
            Some(toasty::stmt::Value::String(title)),
            Some(toasty::stmt::Value::I32(position)),
            Some(toasty::stmt::Value::I32(version)),
            Some(toasty::stmt::Value::String(created_user_id)),
            Some(toasty::stmt::Value::String(created_at)),
            Some(toasty::stmt::Value::String(updated_at)),
        ) = (
            id,
            channel_id,
            title,
            position,
            version,
            created_user_id,
            created_at,
            updated_at,
        )
        else {
            // ページ末尾の空ページ行。次ページはない。
            continue;
        };
        let created_at = created_at
            .parse::<jiff::civil::DateTime>()
            .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
            .to_string();
        let updated_at = updated_at
            .parse::<jiff::civil::DateTime>()
            .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
            .to_string();
        result.push(NoteSummaryResponse {
            id,
            channel_id,
            title,
            position,
            version,
            created_user_id,
            created_at,
            updated_at,
            order_token: current_token.clone(),
        });
    }
    if let Some(expected) = cursor_token
        && expected != current_token
    {
        // 前ページ取得後に並び・構成が変わった。位置カーソルだけでは新しい並びの
        // 続きを安全に特定できないため、最初のページからの再読み込みを要求する。
        return Err(AppError::Conflict(
            "Note order changed; reload from the first page".into(),
        ));
    }
    Ok(Json(ApiResponse::new(result, None::<String>)))
}

#[derive(Deserialize)]
struct DeletedNotesQuery {
    limit: Option<i64>,
    /// `"<deleted_at> <id>"`。前ページの最後の項目から継続する。
    after: Option<String>,
}

/// 削除済み一覧のカーソル `deleted_at` 部分の形式検証。API はこの一覧の `deleted_at` を
/// UTC の `YYYY-MM-DDTHH:MM:SS.mmm` (23 文字、ミリ秒固定) で返すため、カーソルも同じ
/// 形式に限る。文字単位の検証のあと jiff で実在する日時か確認し、不正なら SQL の前に
/// 400 を返す (キャスト失敗を 500 にしない)。
fn is_deleted_at_cursor_datetime(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 23
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'.',
            _ => byte.is_ascii_digit(),
        })
        && jiff::civil::DateTime::strptime("%Y-%m-%dT%H:%M:%S%.3f", value).is_ok()
}

async fn list_deleted(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<DeletedNotesQuery>,
) -> Result<Json<ApiResponse<Vec<DeletedNoteSummaryResponse>>>, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as i32;
    let (after_deleted_at, after_id) = match query.after.as_deref() {
        None | Some("") => (String::new(), String::new()),
        Some(cursor) => match cursor.split_once(' ') {
            // 区切りの空白があれば id は空文字を許す。直接 SQL が空 id を書き得るため。
            Some((deleted_at, id)) if !deleted_at.is_empty() => {
                if !is_deleted_at_cursor_datetime(deleted_at) {
                    return Err(AppError::BadRequest("Invalid deleted note cursor".into()));
                }
                (deleted_at.to_string(), id.to_string())
            }
            _ => return Err(AppError::BadRequest("Invalid deleted note cursor".into())),
        },
    };
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    ensure_channel_member(&mut tx, &actor.id, &channel_id).await?;
    // The eligibility predicate and response body must share one PostgreSQL
    // statement snapshot. Separate SELECTs under READ COMMITTED can combine a
    // pre-delete body with a post-delete eligibility check.
    // 先頭ページと継続ページの SQL を分ける。CASE の内側に置いた行比較は generic plan で
    // 索引の開始条件にならないため、継続ページでは `(deleted_at, id) > (...)` を直接
    // WHERE に置いてカーソル位置から索引走査を開始させる。
    // 一覧は本文を返さない。本文は `GET /notes/deleted/{note_id}` で取得する。
    let rows = if after_deleted_at.is_empty() {
        toasty::sql::query(
            "SELECT id, channel_id, title, position, version, created_user_id, \
                    to_char(created_at, 'YYYY-MM-DD') || 'T' || to_char(created_at, 'HH24:MI:SS.MS'), \
                    to_char(updated_at, 'YYYY-MM-DD') || 'T' || to_char(updated_at, 'HH24:MI:SS.MS'), \
                    to_char(deleted_at, 'YYYY-MM-DD') || 'T' || to_char(deleted_at, 'HH24:MI:SS.MS') \
             FROM channel_notes \
             WHERE channel_id = $1 AND deleted_at IS NOT NULL \
               AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
             ORDER BY deleted_at, id LIMIT $2",
        )
        .bind(channel_id.as_str())
        .bind(limit)
        .exec(&mut tx)
        .await?
    } else {
        toasty::sql::query(
            "SELECT id, channel_id, title, position, version, created_user_id, \
                    to_char(created_at, 'YYYY-MM-DD') || 'T' || to_char(created_at, 'HH24:MI:SS.MS'), \
                    to_char(updated_at, 'YYYY-MM-DD') || 'T' || to_char(updated_at, 'HH24:MI:SS.MS'), \
                    to_char(deleted_at, 'YYYY-MM-DD') || 'T' || to_char(deleted_at, 'HH24:MI:SS.MS') \
             FROM channel_notes \
             WHERE channel_id = $1 AND deleted_at IS NOT NULL \
               AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
               AND (deleted_at, id) > ($2::timestamp(3), $3) \
             ORDER BY deleted_at, id LIMIT $4",
        )
        .bind(channel_id.as_str())
        .bind(after_deleted_at.as_str())
        .bind(after_id.as_str())
        .bind(limit)
        .exec(&mut tx)
        .await?
    };
    let result: Vec<DeletedNoteSummaryResponse> = rows
        .into_iter()
        .map(|row| {
            let toasty::stmt::Value::Record(record) = row else {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected deleted channel note row"
                )));
            };
            let mut fields = record.fields.into_iter();
            let (
                Some(toasty::stmt::Value::String(id)),
                Some(toasty::stmt::Value::String(channel_id)),
                Some(toasty::stmt::Value::String(title)),
                Some(toasty::stmt::Value::I32(position)),
                Some(toasty::stmt::Value::I32(version)),
                Some(toasty::stmt::Value::String(created_user_id)),
                Some(toasty::stmt::Value::String(created_at)),
                Some(toasty::stmt::Value::String(updated_at)),
                Some(toasty::stmt::Value::String(deleted_at)),
            ) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            )
            else {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected deleted channel note columns"
                )));
            };
            let created_at = created_at
                .parse::<jiff::civil::DateTime>()
                .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
                .to_string();
            let updated_at = updated_at
                .parse::<jiff::civil::DateTime>()
                .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
                .to_string();
            // deleted_at はページ継続カーソルの素材で、ミリ秒 3 桁固定の正規形を
            // そのまま返す (jiff の表示整形は末尾の 0 を落とし、カーソル検証が
            // 受け付ける形式と一致しなくなる)。
            Ok(DeletedNoteSummaryResponse {
                id,
                channel_id,
                title,
                position,
                version,
                created_user_id,
                created_at,
                updated_at,
                deleted_at,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(Json(ApiResponse::new(result, None::<String>)))
}

async fn get_deleted(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    ensure_channel_member(&mut tx, &actor.id, &channel_id).await?;
    // 削除済み本文の取得も、24 時間の期限判定と本文を同じステートメントの
    // スナップショットで読む。有効なノート・期限切れ・存在しない場合はすべて 404。
    let rows = toasty::sql::query(
        "SELECT id, channel_id, title, content, position, version, created_user_id, \
                to_char(created_at, 'YYYY-MM-DD') || 'T' || to_char(created_at, 'HH24:MI:SS.MS'), \
                to_char(updated_at, 'YYYY-MM-DD') || 'T' || to_char(updated_at, 'HH24:MI:SS.MS'), \
                to_char(deleted_at, 'YYYY-MM-DD') || 'T' || to_char(deleted_at, 'HH24:MI:SS.MS') \
         FROM channel_notes \
         WHERE id = $1 AND channel_id = $2 AND deleted_at IS NOT NULL \
           AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours'",
    )
    .bind(note_id.as_str())
    .bind(channel_id.as_str())
    .exec(&mut tx)
    .await?;
    let Some(toasty::stmt::Value::Record(record)) = rows.into_iter().next() else {
        return Err(AppError::NotFound("Channel note not found".into()));
    };
    let mut fields = record.fields.into_iter();
    let (
        Some(toasty::stmt::Value::String(id)),
        Some(toasty::stmt::Value::String(channel_id)),
        Some(toasty::stmt::Value::String(title)),
        Some(toasty::stmt::Value::String(content)),
        Some(toasty::stmt::Value::I32(position)),
        Some(toasty::stmt::Value::I32(version)),
        Some(toasty::stmt::Value::String(created_user_id)),
        Some(toasty::stmt::Value::String(created_at)),
        Some(toasty::stmt::Value::String(updated_at)),
        Some(toasty::stmt::Value::String(deleted_at)),
    ) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    )
    else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Unexpected deleted channel note columns"
        )));
    };
    let created_at = created_at
        .parse::<jiff::civil::DateTime>()
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
        .to_string();
    let updated_at = updated_at
        .parse::<jiff::civil::DateTime>()
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
        .to_string();
    Ok(Json(ApiResponse::new(
        NoteResponse {
            id,
            channel_id,
            title,
            content,
            position,
            version,
            created_user_id,
            created_at,
            updated_at,
            deleted_at: Some(deleted_at),
        },
        None::<String>,
    )))
}

async fn ensure_channel_member(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel_id: &str,
) -> Result<(), AppError> {
    let is_member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id)
            .and(ChannelUser::fields().user_id().eq(actor)),
    )
    .first()
    .exec(tx)
    .await?
    .is_some();
    if !is_member {
        return Err(AppError::Forbidden(
            "Join the channel to view deleted notes".into(),
        ));
    }
    Ok(())
}

async fn show(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    let note = find(&mut tx, &channel_id, &note_id).await?;
    Ok(Json(ApiResponse::new(
        NoteResponse::from(note),
        None::<String>,
    )))
}

async fn find(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    note_id: &str,
) -> Result<ChannelNote, AppError> {
    ChannelNote::filter(
        ChannelNote::fields()
            .id()
            .eq(note_id)
            .and(ChannelNote::fields().channel_id().eq(channel_id))
            .and(ChannelNote::fields().deleted_at().is_none()),
    )
    .first()
    .exec(tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Note not found".into()))
}

async fn reorder(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Json(input): Json<ReorderNotesInput>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    // 現在順序は一覧と同じ SQL の ORDER BY position, id で読む。Rust のソートは
    // コードポイント順で、DB の照合順序 (大文字・小文字の順など) と異なり得るため、
    // クライアントが一覧から組み立てた expected_note_ids と一致しなくなる。
    let rows = toasty::sql::query(
        "SELECT id FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL \
         ORDER BY position, id",
    )
    .bind(channel_id.as_str())
    .exec(&mut tx)
    .await?;
    let active_order: Vec<String> = rows
        .into_iter()
        .map(|row| match row {
            toasty::stmt::Value::Record(record) => match record.fields.into_iter().next() {
                Some(toasty::stmt::Value::String(id)) => Ok(id),
                other => Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected channel note id: {other:?}"
                ))),
            },
            other => Err(AppError::Internal(anyhow::anyhow!(
                "Unexpected channel note row: {other:?}"
            ))),
        })
        .collect::<Result<_, _>>()?;
    if active_order != input.expected_note_ids {
        return Err(AppError::Conflict(
            "Note order changed; reload before reordering".into(),
        ));
    }
    let mut active_ids = active_order.clone();
    active_ids.sort();
    let mut requested = input.note_ids.clone();
    requested.sort();
    if active_ids != requested
        || input
            .note_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.note_ids.len()
    {
        return Err(AppError::BadRequest(
            "Order must contain every active note exactly once".into(),
        ));
    }
    for (position, id) in input.note_ids.iter().enumerate() {
        toasty::sql::query(
            "UPDATE channel_notes SET position = $1 WHERE id = $2 AND channel_id = $3",
        )
        .bind(position as i32)
        .bind(id.as_str())
        .bind(channel_id.as_str())
        .exec(&mut tx)
        .await?;
    }
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        serde_json::json!({"updated":true}),
        txid,
    )))
}

async fn restore(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    ensure_note_capacity(&mut tx, &channel_id).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes          SET deleted_at = NULL, version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC'          WHERE id = $2 AND channel_id = $1 AND deleted_at IS NOT NULL            AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours'          RETURNING position",
    )
    .bind(channel_id.as_str())
    .bind(note_id.as_str())
    .exec(&mut tx)
    .await?;
    let Some(toasty::stmt::Value::Record(record)) = rows.first() else {
        return Err(AppError::NotFound("Restorable note not found".into()));
    };
    let Some(toasty::stmt::Value::I32(restored_position)) = record.fields.first() else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Unexpected restored channel note position"
        )));
    };
    // 位置ずらし (`position + 1`) で i32 を溢れさせる行があるか確認する。復元対象
    // 自身はずらしの対象外 (`id <> $3`) なので、復元位置以降の `i32::MAX` 行だけが
    // 該当する。あれば全有効ノート (復元済みノートを含む) を連番に詰め直す。
    // `(position, id)` 順に振り直すため復元ノートの相対位置は保たれ、ずらしも不要になる。
    let overflow_rows = toasty::sql::query(
        "SELECT EXISTS (SELECT 1 FROM channel_notes \
          WHERE channel_id = $1 AND deleted_at IS NULL \
            AND position >= $2 AND id <> $3 AND position = 2147483647)",
    )
    .bind(channel_id.as_str())
    .bind(*restored_position)
    .bind(note_id.as_str())
    .exec(&mut tx)
    .await?;
    let position_overflow = match overflow_rows.first() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.first() {
            Some(toasty::stmt::Value::Bool(overflow)) => *overflow,
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected channel note position overflow check"
                )));
            }
        },
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Missing channel note position overflow check"
            )));
        }
    };
    if position_overflow {
        compact_active_note_positions(&mut tx, &channel_id, Some(&note_id)).await?;
    } else {
        toasty::sql::query(
            "UPDATE channel_notes SET position = position + 1          WHERE channel_id = $1 AND deleted_at IS NULL AND position >= $2 AND id <> $3",
        )
        .bind(channel_id.as_str())
        .bind(*restored_position)
        .bind(note_id.as_str())
        .exec(&mut tx)
        .await?;
    }
    let note = find(&mut tx, &channel_id, &note_id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(NoteResponse::from(note), txid)))
}

async fn create(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Json(input): Json<NoteInput>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    validate(&input)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    ensure_note_capacity(&mut tx, &channel_id).await?;
    let id = Uuid::new_v4().to_string();
    let now = crate::models::now_utc_millis();
    // `+ 1` を SQL で行うと MAX(position) = i32::MAX のとき integer オーバーフローに
    // なるため、最大値をそのまま読んで Rust で判定する。直接 SQL が i32::MAX 近辺の
    // position を書いていた場合は、先に (position, id) 順の連番へ詰め直して相対順序を
    // 保ったまま新しい末尾位置を作る。
    let rows = toasty::sql::query(
        "SELECT COALESCE(MAX(position), -1) AS position FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL",
    )
    .bind(channel_id.as_str())
    .exec(&mut tx)
    .await?;
    let max_position = match rows.first() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.first() {
            Some(toasty::stmt::Value::I32(value)) => *value,
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected channel note max position"
                )));
            }
        },
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Missing channel note max position"
            )));
        }
    };
    let position = match max_position.checked_add(1) {
        Some(next) => next,
        None => compact_active_note_positions(&mut tx, &channel_id, None).await?,
    };
    let note = toasty::create!(ChannelNote {
        id: id.as_str(),
        channel_id: channel_id.as_str(),
        title: input.title.trim(),
        content: input.content.as_str(),
        position,
        deleted_at: None,
        version: 1,
        created_user_id: actor.id.as_str(),
        created_at: now,
        updated_at: now,
    })
    .exec(&mut tx)
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(NoteResponse::from(note), txid)))
}

async fn update(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
    Json(input): Json<UpdateNoteInput>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    validate(&NoteInput {
        title: input.title.clone(),
        content: input.content.clone(),
    })?;
    if input.version < 1 {
        return Err(AppError::BadRequest("Invalid note version".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes SET title = $1, content = $2, version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC' \
         WHERE id = $3 AND channel_id = $4 AND version = $5 AND deleted_at IS NULL RETURNING id",
    )
    .bind(input.title.trim())
    .bind(input.content.as_str())
    .bind(note_id.as_str())
    .bind(channel_id.as_str())
    .bind(input.version)
    .exec(&mut tx)
    .await?;
    if rows.is_empty() {
        return Err(stale_or_missing(&mut tx, &channel_id, &note_id).await?);
    }
    let note = find(&mut tx, &channel_id, &note_id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(NoteResponse::from(note), txid)))
}

async fn stale_or_missing(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    note_id: &str,
) -> Result<AppError, AppError> {
    match find(tx, channel_id, note_id).await {
        Ok(_) => Ok(AppError::Conflict(
            "Note has changed; reload before editing".into(),
        )),
        Err(AppError::NotFound(_)) => Ok(AppError::NotFound("Note not found".into())),
        Err(error) => Err(error),
    }
}

async fn remove(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
    Json(input): Json<DeleteNoteInput>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    note_path(&channel_id, &note_id)?;
    if input.version < 1 {
        return Err(AppError::BadRequest("Invalid note version".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes SET deleted_at = statement_timestamp() AT TIME ZONE 'UTC', version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC' WHERE id = $1 AND channel_id = $2 AND version = $3 AND deleted_at IS NULL RETURNING id",
    )
    .bind(note_id.as_str())
    .bind(channel_id.as_str())
    .bind(input.version)
    .exec(&mut tx)
    .await?;
    if rows.is_empty() {
        return Err(stale_or_missing(&mut tx, &channel_id, &note_id).await?);
    }
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        serde_json::json!({"deleted":true}),
        txid,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_note_title_and_content_limits() {
        assert!(
            validate(&NoteInput {
                title: "  Runbook  ".into(),
                content: "body".into(),
            })
            .is_ok()
        );
        assert!(
            validate(&NoteInput {
                title: "  ".into(),
                content: String::new(),
            })
            .is_err()
        );
        assert!(
            validate(&NoteInput {
                title: "n".repeat(201),
                content: String::new(),
            })
            .is_err()
        );
        assert!(
            validate(&NoteInput {
                title: "n".into(),
                content: "x".repeat(100_001),
            })
            .is_err()
        );
    }
}
