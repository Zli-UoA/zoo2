//! 全文検索（メッセージ・添付ファイル名・チャンネルノート）。設計は docs/search.md。
//!
//! pg_bigm の GIN を（作成月、本文）の複合にしてあり、新しい月から 1 か月ずつ引く。
//! 1 回のリクエストで見る月は最大 [`MONTHS_PER_REQUEST`] か月で、走査はその月々のヒットに限られる。
//! 総履歴には比例しない。K 件に満たなくても、まだ古い月が残っていれば `next_cursor` を返す。
//!
//! 暗号化チャンネルは対象外（docs/e2ee.md：暗号化チャンネルの検索はクライアントだけで行う）。
//! 暗号化に切り替える前の平文のメッセージも、チャンネル単位で除外する。
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{auth::AuthUser, error::AppError, state::AppState};

/// 1 回のリクエストで見る月の数の上限。
pub const MONTHS_PER_REQUEST: i64 = 12;
const MAX_TERMS: usize = 5;
const MAX_TERM_CHARS: usize = 100;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;
const SNIPPET_BEFORE: usize = 40;
const SNIPPET_CHARS: usize = 160;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/search", get(search))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    #[default]
    Messages,
    Files,
    Notes,
}

#[derive(Default, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default, rename = "type")]
    kind: SearchKind,
    channel_id: Option<String>,
    user_id: Option<String>,
    /// この日（UTC、`YYYY-MM-DD`）以降。
    after: Option<String>,
    /// この日（UTC、`YYYY-MM-DD`）より前。
    before: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

/// 次のページの位置。`at`/`id` がなければ、その月の最初（いちばん新しい側）から。
#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Cursor {
    m: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

/// 空白で区切った語。重複を除き、最大 [`MAX_TERMS`] 個。
pub fn parse_terms(q: &str) -> Result<Vec<String>, AppError> {
    let mut terms: Vec<String> = Vec::new();
    for term in q.split_whitespace() {
        if term.chars().count() > MAX_TERM_CHARS {
            return Err(AppError::BadRequest(format!(
                "Each search term must be at most {MAX_TERM_CHARS} characters"
            )));
        }
        if !terms.iter().any(|t| t == term) {
            terms.push(term.to_string());
        }
    }
    if terms.is_empty() {
        return Err(AppError::BadRequest("q is required".into()));
    }
    if terms.len() > MAX_TERMS {
        return Err(AppError::BadRequest(format!(
            "At most {MAX_TERMS} search terms are allowed"
        )));
    }
    Ok(terms)
}

/// 年*12+月。インデックスの式と同じ。
fn month_of(date: jiff::civil::Date) -> i64 {
    i64::from(date.year()) * 12 + i64::from(date.month())
}

fn month_label(month: i64) -> String {
    let (year, m) = ((month - 1).div_euclid(12), (month - 1).rem_euclid(12) + 1);
    format!("{year:04}-{m:02}")
}

fn parse_day(value: Option<&str>, name: &str) -> Result<Option<jiff::civil::Date>, AppError> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<jiff::civil::Date>()
                .map_err(|_| AppError::BadRequest(format!("{name} must be YYYY-MM-DD")))
        })
        .transpose()
}

fn parse_cursor(value: &str) -> Result<Cursor, AppError> {
    let invalid = || AppError::BadRequest("Invalid search cursor".into());
    let cursor: Cursor = serde_json::from_str(value).map_err(|_| invalid())?;
    // 月は SQL で int に変換する。範囲外は DB のエラー（500）ではなく 400 にする。
    if i32::try_from(cursor.m).is_err() {
        return Err(invalid());
    }
    match (&cursor.at, &cursor.id) {
        (None, None) => {}
        (Some(at), Some(_)) => {
            at.parse::<jiff::civil::DateTime>().map_err(|_| invalid())?;
        }
        _ => return Err(invalid()),
    }
    Ok(cursor)
}

/// 最初にヒットした語のまわりを切り出す。英字は大文字・小文字を区別しない（SQL の lower() と同じく ASCII だけ）。
pub fn snippet(text: &str, terms: &[String]) -> String {
    let lower = text.to_ascii_lowercase();
    let start_byte = terms
        .iter()
        .filter_map(|t| lower.find(&t.to_ascii_lowercase()))
        .min()
        .unwrap_or(0);
    let chars_before = text[..start_byte].chars().count();
    let from = chars_before.saturating_sub(SNIPPET_BEFORE);
    let total = text.chars().count();
    let body: String = text.chars().skip(from).take(SNIPPET_CHARS).collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        body,
        if from + SNIPPET_CHARS < total {
            "…"
        } else {
            ""
        }
    )
}

/// 語の数だけ LIKE を並べる。値はすべてバインドする。`first` は最初の語のパラメーター番号。
///
/// 1 文字の語は、pg_bigm がその文字を含むすべてのキーを部分一致で読むため、月の列と突き合わせても
/// 読み飛ばせず、索引の読み取りが総履歴に比例する。そこで 1 文字の語は `|| ''` で索引の条件から外し、
/// 月のキーで絞った行に対するヒープの確認だけにする（走査はその月の行数で頭打ち）。
fn like_clauses(column: &str, terms: &[&str], first: usize) -> String {
    terms
        .iter()
        .enumerate()
        .map(|(i, term)| {
            let target = if term.chars().count() < 2 {
                format!("(lower({column}) || '')")
            } else {
                format!("lower({column})")
            };
            format!(" AND {target} LIKE public.likequery(lower(${}))", first + i)
        })
        .collect()
}

/// スニペットの元。どれかの語が最初に当たった位置の少し前から切り出す（長い本文の後ろで当たっても語を
/// 含み、語の順番にもよらない）。前を省いたら `…` を付ける。語は `$9` から `terms` 個。
fn snippet_source(column: &str, terms: usize) -> String {
    let positions: Vec<String> = (0..terms)
        .map(|i| format!("NULLIF(strpos(lower({column}), lower(${})), 0)", 9 + i))
        .collect();
    let first = format!("COALESCE(LEAST({}), 1)", positions.join(", "));
    format!(
        "(CASE WHEN {first} > 201 THEN '…' ELSE '' END \
          || substr({column}, greatest(1, {first} - 200), 1000))"
    )
}

/// 1 か月分を引く SQL。
///
/// 探す行は（月、本文）の GIN だけで決める。チャンネル・投稿者の条件は `|| ''` を付けて、
/// `idx_messages_channel_id` などの B-tree と BitmapAnd されないようにする。そうしないと、
/// GIN の件数の見積もりが小さいとき、見えるチャンネルの全履歴をインデックスで読む計画になる
/// （予算テストで 100 万件中 22 万行を読んだ）。条件はヒープの行の絞り込みとしてだけ使う。
///パラメーター: $1 月、$2 見えるチャンネル（JSON 配列）、$3 投稿者、
/// $4/$5 keyset（作成日時・ID）、$6/$7 期間、$8 件数、$9 以降が語。
pub fn month_sql(kind: SearchKind, terms: &[&str]) -> String {
    const MONTH: &str = "(date_part('year', {t}.created_at) * 12 + date_part('month', {t}.created_at))::integer = $1::int";
    let common = |t: &str, author: &str| {
        format!(
            "{month} AND ({t}.channel_id || '') = ANY(ARRAY(SELECT json_array_elements_text($2::json))) \
             AND ($3 = '' OR ({t}.{author} || '') = $3) \
             AND ($4 = '' OR ({t}.created_at, {t}.id) < (NULLIF($4, '')::timestamp, $5)) \
             AND ($6 = '' OR {t}.created_at >= NULLIF($6, '')::timestamp) \
             AND ($7 = '' OR {t}.created_at < NULLIF($7, '')::timestamp) \
             ORDER BY {t}.created_at DESC, {t}.id DESC LIMIT $8::int",
            month = MONTH.replace("{t}", t)
        )
    };
    match kind {
        SearchKind::Messages => format!(
            "SELECT json_build_object('id', m.id, 'channel_id', m.channel_id, 'channel_name', c.channel_name, \
               'is_dm', c.is_dm, 'parent_id', m.parent_id, 'user_id', m.user_id, \
               'author', m.display_data->'author', 'emojis', COALESCE(m.display_data->'emojis', '{{}}'::jsonb), \
               'text', {text}, 'created_at', m.created_at)::text \
             FROM (SELECT m.* FROM messages m WHERE m.deleted_at IS NULL AND m.content <> ''{likes} AND {common}) m \
             JOIN channels c ON c.id = m.channel_id ORDER BY m.created_at DESC, m.id DESC",
            likes = like_clauses("m.content", terms, 9),
            text = snippet_source("m.content", terms.len()),
            common = common("m", "user_id"),
        ),
        SearchKind::Files => format!(
            "SELECT json_build_object('id', a.id, 'message_id', a.message_id, 'channel_id', a.channel_id, \
               'channel_name', c.channel_name, 'is_dm', c.is_dm, 'parent_id', m.parent_id, \
               'user_id', a.uploader_id, 'author', m.display_data->'author', \
               'file_name', a.file_name, 'content_type', a.content_type, 'file_size', a.file_size, \
               'width', a.width, 'height', a.height, 'is_sensitive', a.is_sensitive, \
               'text', a.file_name, 'created_at', a.created_at)::text \
             FROM (SELECT a.* FROM attachments a WHERE a.message_id IS NOT NULL{likes} \
               AND EXISTS (SELECT 1 FROM messages live WHERE live.id = a.message_id AND live.deleted_at IS NULL) \
               AND {common}) a \
             JOIN messages m ON m.id = a.message_id JOIN channels c ON c.id = a.channel_id \
             ORDER BY a.created_at DESC, a.id DESC",
            likes = like_clauses("a.file_name", terms, 9),
            common = common("a", "uploader_id"),
        ),
        SearchKind::Notes => format!(
            "SELECT json_build_object('id', n.id, 'channel_id', n.channel_id, 'channel_name', c.channel_name, \
               'is_dm', c.is_dm, 'user_id', n.created_user_id, 'title', n.title, \
               'text', {text}, 'created_at', n.created_at, 'updated_at', n.updated_at)::text \
             FROM (SELECT n.* FROM channel_notes n WHERE n.deleted_at IS NULL{likes} AND {common}) n \
             JOIN channels c ON c.id = n.channel_id ORDER BY n.created_at DESC, n.id DESC",
            likes = like_clauses("(n.title || E'\\n' || n.content)", terms, 9),
            text = snippet_source("(n.title || E'\\n' || n.content)", terms.len()),
            common = common("n", "created_user_id"),
        ),
    }
}

/// 見えるチャンネルと、探す月の範囲。
///
/// 見える: 削除されていない、暗号化されていない、そして（ゲストでない人の公開チャンネル、または所属）。
/// `floor` は見えるチャンネルのうち、最も古いトップレベルのメッセージかチャンネル作成の月の 1 つ前。
/// 添付はメッセージより少し前にアップロードされうるので 1 か月の余裕を持たせる。
/// 計算量は見えるチャンネル数 V に対し O(V log N)（idx_messages_channel_unread を 1 行ずつ引く）。
pub const SCOPE_SQL: &str = "WITH me AS (SELECT COALESCE((SELECT is_guest FROM users WHERE id = $1), true) AS guest), \
  visible AS MATERIALIZED ( \
    SELECT c.id, c.created_at FROM channels c, me \
    WHERE c.deleted_at IS NULL AND NOT c.encrypted AND ($2 = '' OR c.id = $2) \
      AND ((NOT c.is_private AND NOT me.guest) \
        OR EXISTS (SELECT 1 FROM channel_user cu WHERE cu.channel_id = c.id AND cu.user_id = $1)) \
  ) \
  SELECT json_build_object( \
    'ids', COALESCE((SELECT json_agg(id) FROM visible), '[]'::json), \
    'now', (date_part('year', LOCALTIMESTAMP) * 12 + date_part('month', LOCALTIMESTAMP))::bigint, \
    'floor', (SELECT (date_part('year', t) * 12 + date_part('month', t))::bigint - 1 FROM ( \
      SELECT least(min(v.created_at), min(first.created_at)) AS t FROM visible v \
      LEFT JOIN LATERAL (SELECT m.created_at FROM messages m \
        WHERE m.channel_id = v.id AND m.parent_id IS NULL AND m.deleted_at IS NULL \
        ORDER BY m.created_at LIMIT 1) first ON true) bounds) \
  )::text";

async fn search(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let terms = parse_terms(&query.q)?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let after = parse_day(query.after.as_deref(), "after")?;
    let before = parse_day(query.before.as_deref(), "before")?;
    let cursor = query.cursor.as_deref().map(parse_cursor).transpose()?;
    let channel = query.channel_id.as_deref().unwrap_or("");
    let author = query.user_id.as_deref().unwrap_or("");

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 複数の月を同じスナップショットで読む。
    toasty::sql::statement("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .exec(&mut tx)
        .await?;
    // 月・語・空文字で外す条件（`$3 = '' OR …`）の値で計画が変わる。generic plan では月と本文の GIN を
    // 使えず、総履歴を読む経路になりうるので、値を埋めた custom plan だけを使う（予算テストもこの計画を測る）。
    toasty::sql::statement("SET LOCAL plan_cache_mode = force_custom_plan")
        .exec(&mut tx)
        .await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let scope = crate::apps::one(&mut tx, SCOPE_SQL, &[auth_user.id.as_str(), channel]).await?;
    let ids = scope["ids"].to_string();
    let empty = || Json(json!({"items": [], "next_cursor": null, "searched_until": null}));
    let (Some(now), Some(floor)) = (scope["now"].as_i64(), scope["floor"].as_i64()) else {
        tx.commit().await?;
        return Ok(empty());
    };
    if scope["ids"].as_array().is_none_or(Vec::is_empty) {
        tx.commit().await?;
        return Ok(empty());
    }

    // `before` の日を含まない（その日の 0 時より前）。`before` が月初なら前の月から始める。
    let before_month = before.map(|d| month_of(d.yesterday().unwrap_or(d)));
    let floor = after.map_or(floor, |d| floor.max(month_of(d)));
    let start = cursor
        .as_ref()
        .map(|c| c.m)
        .unwrap_or_else(|| before_month.map_or(now, |b| b.min(now)));
    let after_ts = after.map(|d| d.to_string()).unwrap_or_default();
    let before_ts = before.map(|d| d.to_string()).unwrap_or_default();
    let term_refs: Vec<&str> = terms.iter().map(String::as_str).collect();
    let sql = month_sql(query.kind, &term_refs);

    let mut items: Vec<Value> = Vec::new();
    let mut month = start;
    let mut scanned = 0;
    let mut last_month = None;
    while month >= floor && scanned < MONTHS_PER_REQUEST {
        let (at, id) = match &cursor {
            Some(Cursor {
                m,
                at: Some(at),
                id: Some(id),
            }) if *m == month => (at.as_str(), id.as_str()),
            _ => ("", ""),
        };
        let need = (limit + 1 - items.len()).to_string();
        let month_text = month.to_string();
        let mut params: Vec<&str> = vec![
            &month_text,
            &ids,
            author,
            at,
            id,
            &after_ts,
            &before_ts,
            &need,
        ];
        params.extend(terms.iter().map(String::as_str));
        let rows = crate::apps::query(&mut tx, &sql, &params).await?;
        scanned += 1;
        last_month = Some(month);
        items.extend(rows);
        if items.len() > limit {
            break;
        }
        month -= 1;
    }
    tx.commit().await?;

    let next_cursor = if items.len() > limit {
        items.truncate(limit);
        let last = items.last().expect("limit is at least 1");
        Some(Cursor {
            m: month,
            at: last["created_at"].as_str().map(str::to_string),
            id: last["id"].as_str().map(str::to_string),
        })
    } else if month >= floor {
        Some(Cursor {
            m: month,
            at: None,
            id: None,
        })
    } else {
        None
    };
    for item in &mut items {
        if let Some(object) = item.as_object_mut() {
            let text = object
                .remove("text")
                .and_then(|t| t.as_str().map(str::to_string));
            object.insert(
                "snippet".into(),
                Value::String(snippet(text.as_deref().unwrap_or(""), &terms)),
            );
        }
    }
    Ok(Json(json!({
        "items": items,
        "next_cursor": next_cursor.map(|c| serde_json::to_string(&c).expect("cursor serializes")),
        "searched_until": last_month.map(month_label),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_split_deduplicated_and_bounded() {
        assert_eq!(
            parse_terms(" 会議  資料 会議 ").unwrap(),
            vec!["会議", "資料"]
        );
        assert!(parse_terms("   ").is_err());
        assert!(parse_terms("a b c d e f").is_err());
        assert!(parse_terms(&"あ".repeat(MAX_TERM_CHARS + 1)).is_err());
    }

    #[test]
    fn snippet_centers_on_first_hit_case_insensitively() {
        let text = format!("{}Hello World{}", "x".repeat(100), "y".repeat(300));
        let s = snippet(&text, &["world".into()]);
        assert!(s.starts_with('…') && s.ends_with('…'));
        assert!(s.contains("Hello World"));
        assert_eq!(snippet("短い本文", &["本".into()]), "短い本文");
    }

    #[test]
    fn month_labels_and_cursor_roundtrip() {
        assert_eq!(month_label(2026 * 12 + 11), "2026-11");
        assert_eq!(month_label(2026 * 12 + 12), "2026-12");
        let c = parse_cursor(r#"{"m":24323,"at":"2026-11-02T03:04:05.123","id":"x"}"#).unwrap();
        assert_eq!(c.m, 24323);
        assert!(parse_cursor(r#"{"m":1,"at":"2026-11-02T03:04:05"}"#).is_err());
        assert!(parse_cursor("nope").is_err());
    }

    #[test]
    fn month_sql_binds_every_term() {
        let sql = month_sql(SearchKind::Messages, &["会議", "資料", "x"]);
        assert!(sql.contains("lower($9)") && sql.contains("lower($11)"));
        assert!(!sql.contains("$12"));
        // 1 文字の語だけ索引の条件から外す。
        assert!(sql.contains("(lower(m.content) || '') LIKE public.likequery(lower($11))"));
        assert!(sql.contains("lower(m.content) LIKE public.likequery(lower($9))"));
    }

    #[test]
    fn cursor_month_must_fit_in_int() {
        assert!(parse_cursor(r#"{"m":3000000000}"#).is_err());
        assert!(parse_cursor(r#"{"m":24323}"#).is_ok());
    }
}
