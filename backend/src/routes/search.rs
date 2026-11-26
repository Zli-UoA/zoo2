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
/// 送信者・チャンネルの絞り込みで一度に指定できる数。
const MAX_FILTER_IDS: usize = 20;
/// 語の最初のパラメーター番号（それより前は [`month_sql`] の説明を参照）。
const FIRST_TERM: usize = 13;
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
    /// メッセージと添付ファイル名をまとめて、新しい順に（ノートは含めない）。クライアントは明示して使う。
    All,
    /// 既定。`type` を省いた呼び出しの意味は #643 のときのまま変えない（`id` は常にメッセージの ID）。
    #[default]
    Messages,
    Files,
    Notes,
}

impl SearchKind {
    /// 月ごとに流す SQL の種類。`All` はメッセージとファイルの 2 本を流し、時刻順に合わせる。
    pub fn parts(self) -> &'static [SearchKind] {
        match self {
            SearchKind::All => &[SearchKind::Messages, SearchKind::Files],
            SearchKind::Messages => &[SearchKind::Messages],
            SearchKind::Files => &[SearchKind::Files],
            SearchKind::Notes => &[SearchKind::Notes],
        }
    }

    /// 応答の各項目の `kind`。
    fn item_kind(self) -> &'static str {
        match self {
            SearchKind::Files => "file",
            SearchKind::Notes => "note",
            _ => "message",
        }
    }
}

#[derive(Default, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default, rename = "type")]
    kind: SearchKind,
    /// チャンネルの ID。カンマ区切りで複数（どれか）。
    channel_id: Option<String>,
    /// 送信者の ID。カンマ区切りで複数（どれか）。
    user_id: Option<String>,
    /// `true` ならボット（アプリ）の投稿を除く。
    #[serde(default)]
    exclude_bots: bool,
    /// `true` なら、自分が参加中の会話（所属するチャンネルと、参加しているスレッド）に絞る。
    #[serde(default)]
    involving_me: bool,
    /// この時刻以降。`YYYY-MM-DD`（UTC のその日の 0 時）か、オフセット付きの時刻（RFC 3339）。
    /// クライアントは端末の日付の 0 時を、オフセット付きで送る。
    after: Option<String>,
    /// この時刻より前。書き方は `after` と同じ。
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

/// 期間の境界を、UTC の naive な日時（`created_at` と同じ）にする。`YYYY-MM-DD` は UTC のその日の 0 時、
/// オフセット付きの時刻（`2026-11-22T00:00:00+09:00` など）はその瞬間。
fn parse_bound(value: Option<&str>, name: &str) -> Result<Option<jiff::civil::DateTime>, AppError> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| {
            let parsed = if v.len() == 10 {
                v.parse::<jiff::civil::Date>()
                    .ok()
                    .map(|d| d.to_datetime(jiff::civil::Time::midnight()))
            } else {
                v.parse::<jiff::Timestamp>()
                    .ok()
                    .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).datetime())
            };
            parsed.ok_or_else(|| {
                AppError::BadRequest(format!(
                    "{name} must be YYYY-MM-DD or an RFC 3339 timestamp with an offset"
                ))
            })
        })
        .transpose()
}

/// `before` より前の最後の瞬間が入る月。`before` が月初の 0 時ちょうどなら前の月。
fn month_before(before: jiff::civil::DateTime) -> i64 {
    let date = before.date();
    if before.time() == jiff::civil::Time::midnight() {
        month_of(date.yesterday().unwrap_or(date))
    } else {
        month_of(date)
    }
}

/// カンマ区切りの ID を、重複を除いた JSON 配列の文字列にする（空なら `[]`）。
fn parse_ids(value: Option<&str>, name: &str) -> Result<String, AppError> {
    let mut ids: Vec<&str> = Vec::new();
    for id in value.unwrap_or("").split(',').map(str::trim) {
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > MAX_FILTER_IDS {
        return Err(AppError::BadRequest(format!(
            "At most {MAX_FILTER_IDS} values are allowed in {name}"
        )));
    }
    Ok(Value::from(ids).to_string())
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

/// スニペットを SQL で切り出す json の列（`'text', …, 'cut_start', …, 'cut_end', …`）。
///
/// どれかの語が最初に当たった位置の [`SNIPPET_BEFORE`] 文字前から [`SNIPPET_CHARS`] 文字。当たった位置は
/// 検索条件と同じ `lower()` で求め、Rust では探し直さない（`lower()` はロケールによって ASCII 以外も
/// 小文字にするので、Rust の判定とずれることがある）。語の順番にも、長い本文の後ろで当たったかにもよらない。
/// 語は `$13`（[`FIRST_TERM`]）から `terms` 個。
fn snippet_source(column: &str, terms: usize) -> String {
    let positions: Vec<String> = (0..terms)
        .map(|i| {
            format!(
                "NULLIF(strpos(lower({column}), lower(${})), 0)",
                FIRST_TERM + i
            )
        })
        .collect();
    let start = format!(
        "greatest(1, COALESCE(LEAST({}), 1) - {SNIPPET_BEFORE})",
        positions.join(", ")
    );
    format!(
        "'text', substr({column}, {start}, {SNIPPET_CHARS}), \
         'cut_start', {start} > 1, \
         'cut_end', {start} + {SNIPPET_CHARS} <= char_length({column})"
    )
}

/// SQL で切り出した本文をスニペットにする（空白をまとめ、省いた側に `…`）。
fn cut_snippet(text: &str, cut_start: bool, cut_end: bool) -> String {
    format!(
        "{}{}{}",
        if cut_start { "…" } else { "" },
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        if cut_end { "…" } else { "" }
    )
}

/// 自分が参加中の会話の、所属チャンネル以外の条件（[`month_sql`] の $11・$12）。参加しているスレッド・
/// メンションされたスレッドの親か返信か、本文で自分へのメンション（`is_mention`）があるか。
/// スレッドは `IN (SELECT …)` にして、ハッシュの SubPlan で 1 行あたり O(1) で照合する。
const INVOLVING_MESSAGE: &str = " OR COALESCE(m.parent_id, m.id) IN (SELECT json_array_elements_text($11::json)) \
     OR ($12 <> '' AND is_mention(m.content, $12))";

/// 1 か月分を引く SQL。
///
/// 探す行は（月、本文）の GIN だけで決める。チャンネル・投稿者の条件は `|| ''` を付けて、
/// `idx_messages_channel_id` などの B-tree と BitmapAnd されないようにする。そうしないと、
/// GIN の件数の見積もりが小さいとき、見えるチャンネルの全履歴をインデックスで読む計画になる
/// （予算テストで 100 万件中 22 万行を読んだ）。条件はヒープの行の絞り込みとしてだけ使う。
///パラメーター: $1 月、$2 見えるチャンネル（JSON 配列）、$3 投稿者（JSON 配列。`[]` は全員）、
/// $4/$5 keyset（作成日時・ID）、$6/$7 期間、$8 件数、$9 ボットを除くなら `1`、
/// $10 自分が参加中の会話に絞るときの所属チャンネル（JSON 配列。絞らないなら空文字）、
/// $11 参加しているスレッド・メンションされたスレッドのうち、所属しないチャンネルの親の ID（JSON 配列）、
/// $12 本文のメンションを探す自分の `user_name`（絞らないなら空文字）、$13 以降が語。
///
/// ボットかどうかは、行の投稿者の表示データ（`display_data->'author'->>'is_bot'`）で見る。ユーザーの表を
/// 行ごとに引かない。ノートはアプリが書けないので、ボットの条件を付けない。
/// 所属・スレッド・投稿者の条件は、GIN で見つけた行に対する絞り込み。所属とスレッドは `IN (SELECT …)` の
/// ハッシュの SubPlan で、1 回作れば行ごとに O(1)（所属チャンネル数 M・スレッド数 T は作るときに 1 回だけ）。
/// 投稿者は配列と比べる（[`MAX_FILTER_IDS`] まで）。本文のメンションは候補の行ごとに正規表現で見る。
/// `kind` は `SearchKind::parts` の 1 つ（`All` は渡さない）。
pub fn month_sql(kind: SearchKind, terms: &[&str]) -> String {
    assert_ne!(
        kind,
        SearchKind::All,
        "month_sql takes one part of SearchKind::All"
    );
    const MONTH: &str = "(date_part('year', {t}.created_at) * 12 + date_part('month', {t}.created_at))::integer = $1::int";
    let common = |t: &str, author: &str, involving: &str| {
        format!(
            "{month} AND ({t}.channel_id || '') = ANY(ARRAY(SELECT json_array_elements_text($2::json))) \
             AND ($3 = '[]' OR ({t}.{author} || '') = ANY(ARRAY(SELECT json_array_elements_text($3::json)))) \
             AND ($10 = '' OR ({t}.channel_id || '') IN (SELECT json_array_elements_text(NULLIF($10, '')::json)){involving}) \
             AND ($4 = '' OR ({t}.created_at, {t}.id) < (NULLIF($4, '')::timestamp, $5)) \
             AND ($6 = '' OR {t}.created_at >= NULLIF($6, '')::timestamp) \
             AND ($7 = '' OR {t}.created_at < NULLIF($7, '')::timestamp) \
             ORDER BY {t}.created_at DESC, {t}.id DESC LIMIT $8::int",
            month = MONTH.replace("{t}", t)
        )
    };
    match kind {
        SearchKind::All => unreachable!(),
        SearchKind::Messages => format!(
            "SELECT json_build_object('id', m.id, 'channel_id', m.channel_id, 'channel_name', c.channel_name, \
               'is_dm', c.is_dm, 'parent_id', m.parent_id, 'user_id', m.user_id, \
               'author', m.display_data->'author', 'emojis', COALESCE(m.display_data->'emojis', '{{}}'::jsonb), \
               {text}, 'created_at', m.created_at)::text \
             FROM (SELECT m.* FROM messages m WHERE m.deleted_at IS NULL AND m.content <> ''{likes} \
               AND ($9 = '' OR NOT COALESCE((m.display_data->'author'->>'is_bot')::boolean, false)) AND {common}) m \
             JOIN channels c ON c.id = m.channel_id ORDER BY m.created_at DESC, m.id DESC",
            likes = like_clauses("m.content", terms, FIRST_TERM),
            text = snippet_source("m.content", terms.len()),
            common = common("m", "user_id", INVOLVING_MESSAGE),
        ),
        SearchKind::Files => format!(
            "SELECT json_build_object('id', a.id, 'message_id', a.message_id, 'channel_id', a.channel_id, \
               'channel_name', c.channel_name, 'is_dm', c.is_dm, 'parent_id', m.parent_id, \
               'user_id', a.uploader_id, 'author', m.display_data->'author', \
               'file_name', a.file_name, 'content_type', a.content_type, 'file_size', a.file_size, \
               'width', a.width, 'height', a.height, 'is_sensitive', a.is_sensitive, \
               {text}, 'created_at', a.created_at)::text \
             FROM (SELECT a.* FROM attachments a WHERE a.message_id IS NOT NULL{likes} \
               AND EXISTS (SELECT 1 FROM messages live WHERE live.id = a.message_id AND live.deleted_at IS NULL \
                 AND ($9 = '' OR NOT COALESCE((live.display_data->'author'->>'is_bot')::boolean, false)) \
                 AND ($10 = '' OR (a.channel_id || '') IN (SELECT json_array_elements_text(NULLIF($10, '')::json)){file_thread})) \
               AND {common}) a \
             JOIN messages m ON m.id = a.message_id JOIN channels c ON c.id = a.channel_id \
             ORDER BY a.created_at DESC, a.id DESC",
            likes = like_clauses("a.file_name", terms, FIRST_TERM),
            text = snippet_source("a.file_name", terms.len()),
            // 所属とスレッドの条件は、添付の親のメッセージ（`live`）で見る。
            common = common("a", "uploader_id", " OR true"),
            file_thread = INVOLVING_MESSAGE.replace("m.", "live."),
        ),
        SearchKind::Notes => format!(
            "SELECT json_build_object('id', n.id, 'channel_id', n.channel_id, 'channel_name', c.channel_name, \
               'is_dm', c.is_dm, 'user_id', n.created_user_id, 'title', n.title, \
               {text}, 'created_at', n.created_at, 'updated_at', n.updated_at)::text \
             FROM (SELECT n.* FROM channel_notes n WHERE n.deleted_at IS NULL{likes} AND {common}) n \
             JOIN channels c ON c.id = n.channel_id ORDER BY n.created_at DESC, n.id DESC",
            likes = like_clauses("(n.title || E'\\n' || n.content)", terms, FIRST_TERM),
            text = snippet_source("(n.title || E'\\n' || n.content)", terms.len()),
            common = common("n", "created_user_id", ""),
        ),
    }
}

/// 見えるチャンネルと、探す月の範囲。
///
/// パラメーター: $1 閲覧者、$2 チャンネルの絞り込み（JSON 配列。`[]` は全部）、$3 自分が参加中の会話に
/// 絞るなら `1`。
///
/// 見える: 削除されていない、暗号化されていない、そして（ゲストでない人の公開チャンネル、または所属）。
/// 参加中の会話に絞るときは、見えるチャンネルのうち所属するもの（DM を含む）を `members` に、参加している
/// スレッドかメンションされたスレッド（メンションは正本の通知 `notification_events` の `mention` で引く）の
/// うち、所属しないチャンネルの親を `threads` に、自分の `user_name` を `user_name` に入れ、[`month_sql`] の
/// 行の条件に渡す。メンションの通知はチャンネルのメンバーにしか作られないので、所属しないチャンネルの
/// 本文のメンションは `is_mention` で見る。そのため探すチャンネル（`ids`）は見えるチャンネル全部のまま。
/// `floor` は見えるチャンネルのうち、最も古い生きているメッセージ（トップレベルと返信）かチャンネル作成の月の
/// 1 つ前。親が削除されても返信は残り、取り込みではチャンネルの作成日時より古いこともあるので、返信も入れる。
/// 添付はメッセージより少し前にアップロードされうるので 1 か月の余裕を持たせる。
/// 計算量は見えるチャンネル数 V に対し O(V log N)（idx_messages_channel_unread と
/// messages_search_reply_floor を、チャンネルごとに 1 行ずつ引く）。参加中に絞るときは、閲覧者の参加
/// スレッド数 T とメンションの通知数 N_m を加えて O(V log N + T + N_m)（thread_participants_joined と
/// notification_events_user_mention、親は主キーで引く）。メンションは部分インデックスで引くので、閲覧者の
/// ほかの通知（DM・リアクションなど）の数には比例しない。
pub const SCOPE_SQL: &str = "WITH me AS (SELECT COALESCE((SELECT is_guest FROM users WHERE id = $1), true) AS guest), \
  visible AS MATERIALIZED ( \
    SELECT c.id, c.created_at FROM channels c, me \
    WHERE c.deleted_at IS NULL AND NOT c.encrypted \
      AND ($2 = '[]' OR c.id = ANY(ARRAY(SELECT json_array_elements_text($2::json)))) \
      AND ((NOT c.is_private AND NOT me.guest) \
        OR EXISTS (SELECT 1 FROM channel_user cu WHERE cu.channel_id = c.id AND cu.user_id = $1)) \
  ), \
  members AS MATERIALIZED ( \
    SELECT v.id FROM visible v WHERE $3 <> '' \
      AND EXISTS (SELECT 1 FROM channel_user cu WHERE cu.channel_id = v.id AND cu.user_id = $1) \
  ), \
  threads AS MATERIALIZED ( \
    SELECT DISTINCT root.id, root.channel_id FROM ( \
      SELECT tp.thread_id AS id FROM thread_participants tp \
        WHERE $3 <> '' AND tp.user_id = $1 AND tp.left_at IS NULL \
      UNION \
      SELECT COALESCE(m.parent_id, m.id) FROM notification_events n JOIN messages m ON m.id = n.message_id \
        WHERE $3 <> '' AND n.user_id = $1 AND n.notification_type = 'mention' \
    ) t JOIN messages root ON root.id = t.id \
    WHERE root.channel_id IN (SELECT id FROM visible) AND root.channel_id NOT IN (SELECT id FROM members) \
  ) \
  SELECT json_build_object( \
    'ids', COALESCE((SELECT json_agg(id) FROM visible), '[]'::json), \
    'members', COALESCE((SELECT json_agg(id) FROM members), '[]'::json), \
    'threads', COALESCE((SELECT json_agg(id) FROM threads), '[]'::json), \
    'user_name', CASE WHEN $3 <> '' THEN (SELECT user_name FROM users WHERE id = $1) END, \
    'now', (date_part('year', LOCALTIMESTAMP) * 12 + date_part('month', LOCALTIMESTAMP))::bigint, \
    'floor', (SELECT (date_part('year', t) * 12 + date_part('month', t))::bigint - 1 FROM ( \
      SELECT least(min(v.created_at), min(first.created_at), min(reply.created_at)) AS t FROM visible v \
      LEFT JOIN LATERAL (SELECT m.created_at FROM messages m \
        WHERE m.channel_id = v.id AND m.parent_id IS NULL AND m.deleted_at IS NULL \
        ORDER BY m.created_at LIMIT 1) first ON true \
      LEFT JOIN LATERAL (SELECT m.created_at FROM messages m \
        WHERE m.channel_id = v.id AND m.parent_id IS NOT NULL AND m.deleted_at IS NULL \
        ORDER BY m.created_at LIMIT 1) reply ON true) bounds) \
  )::text";

async fn search(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let terms = parse_terms(&query.q)?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let after = parse_bound(query.after.as_deref(), "after")?;
    let before = parse_bound(query.before.as_deref(), "before")?;
    let cursor = query.cursor.as_deref().map(parse_cursor).transpose()?;
    let channels = parse_ids(query.channel_id.as_deref(), "channel_id")?;
    let authors = parse_ids(query.user_id.as_deref(), "user_id")?;
    let involving = if query.involving_me { "1" } else { "" };
    let no_bots = if query.exclude_bots { "1" } else { "" };

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
    let scope = crate::apps::one(
        &mut tx,
        SCOPE_SQL,
        &[auth_user.id.as_str(), channels.as_str(), involving],
    )
    .await?;
    let ids = scope["ids"].to_string();
    // 参加中に絞らないときは空文字（行の条件を外す）。
    let members = if query.involving_me {
        scope["members"].to_string()
    } else {
        String::new()
    };
    let threads = scope["threads"].to_string();
    let my_name = scope["user_name"].as_str().unwrap_or("").to_string();
    let empty = || Json(json!({"items": [], "next_cursor": null, "searched_until": null}));
    let (Some(now), Some(floor)) = (scope["now"].as_i64(), scope["floor"].as_i64()) else {
        tx.commit().await?;
        return Ok(empty());
    };
    if scope["ids"].as_array().is_none_or(Vec::is_empty) {
        tx.commit().await?;
        return Ok(empty());
    }

    // `before` の瞬間を含まない。`before` が月初の 0 時ちょうどなら前の月から始める。
    let before_month = before.map(month_before);
    let floor = after.map_or(floor, |d| floor.max(month_of(d.date())));
    let start = cursor
        .as_ref()
        .map(|c| c.m)
        .unwrap_or_else(|| before_month.map_or(now, |b| b.min(now)));
    let after_ts = after.map(|d| d.to_string()).unwrap_or_default();
    let before_ts = before.map(|d| d.to_string()).unwrap_or_default();
    let term_refs: Vec<&str> = terms.iter().map(String::as_str).collect();
    let parts: Vec<(SearchKind, String)> = query
        .kind
        .parts()
        .iter()
        .map(|&part| (part, month_sql(part, &term_refs)))
        .collect();

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
            &authors,
            at,
            id,
            &after_ts,
            &before_ts,
            &need,
            no_bots,
            &members,
            &threads,
            &my_name,
        ];
        params.extend(terms.iter().map(String::as_str));
        // 種類ごとに、この月の上位 need 件。合わせて (created_at, id) の新しい順に並べれば、
        // 種類をまたいでも同じ keyset で続きを取れる（月の中で閉じるので、月の順は崩れない）。
        let mut rows = Vec::new();
        for (part, sql) in &parts {
            for mut row in crate::apps::query(&mut tx, sql, &params).await? {
                if let Some(object) = row.as_object_mut() {
                    object.insert("kind".into(), Value::from(part.item_kind()));
                }
                rows.push(row);
            }
        }
        if parts.len() > 1 {
            let key = |row: &Value| {
                (
                    row["created_at"].as_str().unwrap_or("").to_string(),
                    row["id"].as_str().unwrap_or("").to_string(),
                )
            };
            rows.sort_by_key(|row| std::cmp::Reverse(key(row)));
        }
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
                .and_then(|t| t.as_str().map(str::to_string))
                .unwrap_or_default();
            // どの種類も SQL が当たった位置で切り出してある（一致の判定は SQL の lower() だけ）。
            let flag = |value: Option<Value>| value.and_then(|v| v.as_bool()).unwrap_or(false);
            let value = cut_snippet(
                &text,
                flag(object.remove("cut_start")),
                flag(object.remove("cut_end")),
            );
            object.insert("snippet".into(), Value::String(value));
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
    fn cut_snippet_marks_trimmed_sides_and_collapses_whitespace() {
        assert_eq!(cut_snippet("会議の\n  資料", true, false), "…会議の 資料");
        assert_eq!(cut_snippet("短い本文", false, false), "短い本文");
        assert_eq!(cut_snippet("途中", true, true), "…途中…");
    }

    #[test]
    fn filter_ids_are_split_deduplicated_and_bounded() {
        assert_eq!(parse_ids(None, "user_id").unwrap(), "[]");
        assert_eq!(
            parse_ids(Some(" a, b,,a "), "user_id").unwrap(),
            r#"["a","b"]"#
        );
        let many: Vec<String> = (0..=MAX_FILTER_IDS).map(|i| i.to_string()).collect();
        assert!(parse_ids(Some(&many.join(",")), "user_id").is_err());
    }

    #[test]
    fn bounds_accept_utc_days_and_offset_timestamps() {
        let at = |v: &str| parse_bound(Some(v), "after").unwrap().unwrap().to_string();
        assert_eq!(at("2026-11-22"), "2026-11-22T00:00:00");
        // 端末の日付の 0 時（JST）は、UTC の前日 15 時。
        assert_eq!(at("2026-11-22T00:00:00+09:00"), "2026-11-21T15:00:00");
        assert_eq!(at("2026-11-22T00:00:00-05:00"), "2026-11-22T05:00:00");
        assert_eq!(at("2026-11-22T00:00:00Z"), "2026-11-22T00:00:00");
        assert!(parse_bound(Some(""), "after").unwrap().is_none());
        for bad in ["2026-13-01", "2026-11-22T00:00:00", "nope"] {
            assert!(parse_bound(Some(bad), "after").is_err(), "{bad}");
        }
        let month = |v: &str| month_before(parse_bound(Some(v), "before").unwrap().unwrap());
        assert_eq!(month("2026-12-01"), 2026 * 12 + 11);
        // JST の 12/1 の 0 時は UTC の 11/30 15 時なので、11 月。
        assert_eq!(month("2026-12-01T00:00:00+09:00"), 2026 * 12 + 11);
        // UTC の 12/1 の 9 時より前は、12 月の 0 時〜9 時も含む。
        assert_eq!(month("2026-12-01T00:00:00-09:00"), 2026 * 12 + 12);
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
        assert!(sql.contains("lower($13)") && sql.contains("lower($15)"));
        assert!(!sql.contains("$16"));
        // 1 文字の語だけ索引の条件から外す。
        assert!(sql.contains("(lower(m.content) || '') LIKE public.likequery(lower($15))"));
        assert!(sql.contains("lower(m.content) LIKE public.likequery(lower($13))"));
    }

    #[test]
    fn cursor_month_must_fit_in_int() {
        assert!(parse_cursor(r#"{"m":3000000000}"#).is_err());
        assert!(parse_cursor(r#"{"m":24323}"#).is_ok());
    }
}
