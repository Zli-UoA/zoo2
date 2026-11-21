//! Disposable migrated DB only: SEARCH_SCALE_ROWS=300000/1000000 and DATABASE_URL.
//!
//! 検索（routes/search.rs）の各 SQL の走査量を測る。どの月も R 件前後で、履歴（N − R 件）は
//! 同じ密度で先月から過去へ延びる（30 万件で約 14 か月、100 万件で約 49 か月）。1 か月分の SQL の走査が今月のヒットだけに
//! 収まり、履歴を 3.3 倍にしても増えないことを、infra/db/compare-search-work.py が比べる。
use backend::routes::search::{SCOPE_SQL, SearchKind, month_sql};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_postgres::{NoTls, SimpleQueryMessage};

/// 今月の行数（メッセージ）。添付とノートは R/10・R/40。
const R: usize = 20000;
const PUBLIC: usize = 300;
const PRIVATE: usize = 100;
const ENCRYPTED: usize = 20;
const ACTOR: &str = "550e8400-e29b-41d4-a716-4466554400b1";
const NOISE: &str = "550e8400-e29b-41d4-a716-4466554400b2";
/// 所属のないゲスト（見えるチャンネルが 0）。
const LONELY: &str = "550e8400-e29b-41d4-a716-4466554400b3";
/// actor が所属しない非公開チャンネル。ここにだけ「機密語」を置く（見えない結果だけに当たる語）。
const HIDDEN: &str = "sc-301";

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
/// `$n` をリテラルに置き換える（大きい番号から。カスタムプランと同じ計画になる）。
fn bind(sql: &str, params: &[String]) -> String {
    let mut out = sql.to_string();
    for (i, value) in params.iter().enumerate().rev() {
        out = out.replace(&format!("${}", i + 1), &quote(value));
    }
    out
}
fn scan(node: &Value, relations: &mut std::collections::BTreeMap<String, f64>) {
    let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
    let work = (node["Actual Rows"].as_f64().unwrap_or(0.)
        + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
        + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
        * loops;
    let name = node["Relation Name"]
        .as_str()
        .or_else(|| node["Index Name"].as_str());
    if let Some(name) = name {
        *relations.entry(name.into()).or_default() += work;
    }
    // 結合で捨てた行と、ソートの入力（メモリ内で済んだものも）も、表の走査と同じ上限で見る。
    let removed = node["Rows Removed by Join Filter"].as_f64().unwrap_or(0.) * loops;
    if removed > 0. {
        *relations.entry("join-filter-removed".into()).or_default() += removed;
    }
    if matches!(
        node["Node Type"].as_str(),
        Some("Sort" | "Incremental Sort")
    ) {
        let input: f64 = node["Plans"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| {
                child["Actual Rows"].as_f64().unwrap_or(0.)
                    * child["Actual Loops"].as_f64().unwrap_or(0.)
            })
            .sum();
        *relations.entry("sort-input".into()).or_default() += input;
    }
    assert!(
        !node["Sort Method"]
            .as_str()
            .unwrap_or("")
            .contains("external")
    );
    if let Some(children) = node["Plans"].as_array() {
        for child in children {
            scan(child, relations)
        }
    }
}
async fn client(url: &str) -> anyhow::Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(async move { connection.await.unwrap() });
    Ok(client)
}

/// 1 行目に 1 列だけ返す SQL の値。
async fn one(db: &tokio_postgres::Client, sql: &str) -> anyhow::Result<String> {
    let rows = db.simple_query(sql).await?;
    Ok(rows
        .iter()
        .find_map(|r| match r {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
            _ => None,
        })
        .unwrap_or_default())
}

#[tokio::test]
#[ignore = "requires an empty migrated disposable database and SEARCH_SCALE_ROWS"]
async fn search_queries_have_bounded_work() -> anyhow::Result<()> {
    // ここで測るのは値を埋めた custom plan。API も custom plan だけを使うことを確かめる（generic plan は測らない）。
    assert!(
        include_str!("../src/routes/search.rs")
            .contains("SET LOCAL plan_cache_mode = force_custom_plan"),
        "search must force custom plans to match what this budget measures"
    );
    let n: usize = std::env::var("SEARCH_SCALE_ROWS")?.parse()?;
    assert!(n >= 300000);
    let url = std::env::var("DATABASE_URL")?;
    let db = client(&url).await?;
    let history = n - R;
    // 本文の分布（今月も履歴も同じ）: 30% に「です」、2% に「会議」、今月の 1 件だけ「レア語」。
    // 7 行に 1 行は削除済み。チャンネルは公開・非公開（actor は 1/5 だけ所属）・暗号化を巡回する。
    let content = "CASE WHEN i%10<3 THEN 'よろしくお願いします、です '||i WHEN i%50 IN (3,4) THEN '明日の会議の資料 '||i ELSE 'hello world message '||i END";
    let channels = PUBLIC + PRIVATE + ENCRYPTED;
    db.batch_execute(&format!(r#"
SET session_replication_role = replica;
INSERT INTO users(id,user_name,display_name) VALUES('{ACTOR}','search-actor','A'),('{NOISE}','search-noise','N');
INSERT INTO users(id,user_name,display_name,is_guest) VALUES('{LONELY}','search-lonely','G',true);
INSERT INTO channels(id,channel_name,created_user_id,is_private,encrypted,created_at)
SELECT 'sc-'||i,'search-'||i,'{NOISE}',i>{PUBLIC},i>{PUBLIC}+{PRIVATE},LOCALTIMESTAMP-interval '60 months' FROM generate_series(1,{channels}) i;
INSERT INTO channel_user(channel_id,user_id,joined_at)
SELECT 'sc-'||i,'{ACTOR}',LOCALTIMESTAMP FROM generate_series(1,{channels}) i WHERE i<=30 OR (i>{PUBLIC} AND i%5=0);
-- 今月（R 件）。月初から 50ms 間隔なので、月の 0:17 以降に実行すれば今月に収まる。
INSERT INTO messages(id,user_id,channel_id,content,created_at,deleted_at)
SELECT 'r-'||lpad(i::text,8,'0'),CASE WHEN i%2=0 THEN '{ACTOR}' ELSE '{NOISE}' END,'sc-'||(1+i%{channels}),
  CASE WHEN i=8 THEN 'とてもレア語な話' ELSE {content} END,
  date_trunc('month',LOCALTIMESTAMP)+i*interval '50 milliseconds',
  CASE WHEN i%7=0 THEN LOCALTIMESTAMP END
FROM generate_series(1,{R}) i;
-- 履歴（N − R 件）。今月と同じ密度（30 日で R 件）で、先月から過去へ延ばす。
INSERT INTO messages(id,user_id,channel_id,content,created_at,deleted_at)
SELECT 'h-'||lpad(i::text,8,'0'),'{NOISE}','sc-'||(1+i%{channels}),{content},
  date_trunc('month',LOCALTIMESTAMP)-interval '1 second'-i*(interval '30 days'/{R}),
  CASE WHEN i%7=0 THEN LOCALTIMESTAMP END
FROM generate_series(1,{history}) i;
-- 見えないチャンネルにだけある語。今月 300 件と、先月までの 6 か月に 1200 件（規模によらず同じ）。
INSERT INTO messages(id,user_id,channel_id,content,created_at)
SELECT 'x-'||lpad(i::text,8,'0'),'{NOISE}','{HIDDEN}','機密語の記録 '||i,
  CASE WHEN i<=300 THEN date_trunc('month',LOCALTIMESTAMP)+i*interval '1 second'
       ELSE date_trunc('month',LOCALTIMESTAMP)-i*interval '3 hours' END
FROM generate_series(1,1500) i;
INSERT INTO attachments(id,message_id,uploader_id,channel_id,file_name,file_size,content_type,storage_key,created_at)
SELECT 'a'||m.id,m.id,m.user_id,m.channel_id,CASE WHEN i%3=0 THEN '議事録_'||i||'.pdf' ELSE 'photo_'||i||'.jpg' END,10,'application/pdf','k'||m.id,m.created_at
FROM (SELECT row_number() OVER () i,* FROM messages WHERE id LIKE 'r-%' OR id LIKE 'h-%') m WHERE m.i%10=0;
INSERT INTO channel_notes(id,channel_id,title,content,created_user_id,created_at)
SELECT 'n'||m.id,m.channel_id,'メモ '||m.i,CASE WHEN m.i%3=0 THEN '障害対応の手順 '||m.i ELSE '議事メモ '||m.i END,m.user_id,m.created_at
FROM (SELECT row_number() OVER () i,* FROM messages WHERE id LIKE 'r-%' OR id LIKE 'h-%') m WHERE m.i%40=0;
SET session_replication_role = DEFAULT;
"#)).await?;
    // 1 文ずつ送る（複数の文はまとめて暗黙のトランザクションになり、VACUUM が通らない）。
    db.batch_execute("VACUUM ANALYZE").await?;
    // 書き込み直後（VACUUM 前）の行も測る。今月に 500 件を、規模によらず同じだけ公開チャンネルに足す（トリガーは動かす）。
    // 検索用の索引は fastupdate = off なので pending list はなく、足した行は索引に直接入っている。
    db.batch_execute(&format!(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at)
         SELECT 'w-'||lpad(i::text,8,'0'),'{NOISE}','sc-'||(1+i%{PUBLIC}),
           CASE WHEN i%10<3 THEN '書き込み直後です '||i ELSE 'fresh row '||i END,
           date_trunc('month',LOCALTIMESTAMP)+interval '20 minutes'+i*interval '10 milliseconds'
         FROM generate_series(1,500) i;"
    ))
    .await?;
    let pending: String = one(
        &db,
        "SELECT string_agg(c.relname||'='||coalesce(array_to_string(c.reloptions,','),''),' ' ORDER BY c.relname)
         FROM pg_class c WHERE c.relname IN ('messages_search_month','attachments_search_month','channel_notes_search_month')",
    )
    .await?;
    assert_eq!(
        pending.matches("fastupdate=off").count(),
        3,
        "search indexes must not keep a GIN pending list: {pending}"
    );

    let scope: Value =
        serde_json::from_str(&one(&db, &bind(SCOPE_SQL, &[ACTOR.into(), String::new()])).await?)?;
    let ids = scope["ids"].to_string();
    let visible = scope["ids"].as_array().unwrap().len();
    assert_eq!(
        visible,
        PUBLIC + PRIVATE / 5,
        "public + joined private, no encrypted"
    );
    let now = scope["now"].as_i64().unwrap();
    // 最古のチャンネル作成（60 か月前）が下限になる。
    assert!(scope["floor"].as_i64().unwrap() <= now - 60);

    // 今月の、対象の表のヒット数（全チャンネル・削除済みを除く）。走査の上限に使う。
    let hits = |table: &str, column: &str, terms: &[&str]| {
        let likes: String = terms
            .iter()
            .map(|t| {
                format!(
                    " AND lower({column}) LIKE public.likequery(lower({}))",
                    quote(t)
                )
            })
            .collect();
        let live = match table {
            "messages" => "deleted_at IS NULL AND content<>''",
            "attachments" => "message_id IS NOT NULL",
            _ => "deleted_at IS NULL",
        };
        format!(
            "SELECT count(*)::text FROM {table} WHERE {live} AND (date_part('year',created_at)*12+date_part('month',created_at))::integer={now}{likes}"
        )
    };
    // 今月のいちばん古い行の直後を keyset にした、深いページ。
    let deep = one(&db, "SELECT json_build_array(created_at::text, id)::text FROM messages WHERE id LIKE 'r-%' AND deleted_at IS NULL ORDER BY created_at, id OFFSET 30 LIMIT 1").await?;
    let deep: Vec<String> = serde_json::from_str(&deep)?;
    let mut queries: Vec<(String, String, f64)> = Vec::new();
    let scope_bound = (3 * channels + 3 * visible + 64) as f64;
    queries.push((
        "scope".into(),
        bind(SCOPE_SQL, &[ACTOR.into(), String::new()]),
        scope_bound,
    ));
    queries.push((
        "scope-guest-without-channels".into(),
        bind(SCOPE_SQL, &[LONELY.into(), String::new()]),
        (3 * channels + 64) as f64,
    ));
    queries.push((
        "scope-channel".into(),
        bind(SCOPE_SQL, &[ACTOR.into(), "sc-1".into()]),
        64.,
    ));
    // 名前、種類、表、列、語、投稿者、keyset（作成日時・ID）。
    type Case<'a> = (
        &'a str,
        SearchKind,
        &'a str,
        &'a str,
        Vec<&'a str>,
        &'a str,
        [&'a str; 2],
    );
    let cases: Vec<Case> = vec![
        (
            "messages-common",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["です"],
            "",
            ["", ""],
        ),
        (
            "messages-two-terms",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["よろしく", "です"],
            "",
            ["", ""],
        ),
        (
            "messages-one-char",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["す"],
            "",
            ["", ""],
        ),
        (
            "messages-rare",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["レア語"],
            "",
            ["", ""],
        ),
        (
            "messages-none",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["存在しない語"],
            "",
            ["", ""],
        ),
        (
            "messages-author",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["会議"],
            ACTOR,
            ["", ""],
        ),
        (
            "messages-deep-page",
            SearchKind::Messages,
            "messages",
            "content",
            vec!["hello"],
            "",
            [deep[0].as_str(), deep[1].as_str()],
        ),
        (
            "files",
            SearchKind::Files,
            "attachments",
            "file_name",
            vec!["議事録", "pdf"],
            "",
            ["", ""],
        ),
        (
            "notes",
            SearchKind::Notes,
            "channel_notes",
            "(title || E'\\n' || content)",
            vec!["障害"],
            "",
            ["", ""],
        ),
        (
            "files-one-char",
            SearchKind::Files,
            "attachments",
            "file_name",
            vec!["議"],
            "",
            ["", ""],
        ),
        (
            "notes-one-char",
            SearchKind::Notes,
            "channel_notes",
            "(title || E'\\n' || content)",
            vec!["障"],
            "",
            ["", ""],
        ),
    ];
    // 1 文字の語は索引の条件に入れないので、その月の生きている行をすべて確かめる（表ごとの月内の行数が上限）。
    for (name, kind, table, column, terms, author, [at, id]) in &cases {
        let h: f64 = if terms.iter().any(|t| t.chars().count() < 2) {
            one(&db, &hits(table, column, &[])).await?.parse()?
        } else {
            one(&db, &hits(table, column, terms)).await?.parse()?
        };
        let mut params = vec![
            now.to_string(),
            ids.clone(),
            author.to_string(),
            at.to_string(),
            id.to_string(),
            String::new(),
            String::new(),
            "21".into(),
        ];
        params.extend(terms.iter().map(|t| t.to_string()));
        // 対象の表は「今月のヒット」の定数倍、それ以外（JOIN 先）はページの件数に比例。
        queries.push((
            name.to_string(),
            bind(&month_sql(*kind, terms), &params),
            3. * h + 64.,
        ));
    }
    // 履歴より前の空の月: インデックスを引くだけ。
    let mut empty = vec![
        (now - 59).to_string(),
        ids.clone(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "21".into(),
        "です".into(),
    ];
    queries.push((
        "messages-empty-month".into(),
        bind(&month_sql(SearchKind::Messages, &["です"]), &empty),
        8.,
    ));
    empty[0] = (now - 1).to_string();
    let h: f64 = one(
        &db,
        &hits("messages", "content", &["です"])
            .replace(&format!("={now}"), &format!("={}", now - 1)),
    )
    .await?
    .parse()?;
    queries.push((
        "messages-previous-month".into(),
        bind(&month_sql(SearchKind::Messages, &["です"]), &empty),
        3. * h + 64.,
    ));

    // 見えない結果だけに当たる語（その月のヒットは見えないチャンネルの 300 件）と、
    // 見えるチャンネルが 0 の閲覧者（API はここで返すが、SQL 単体でも上限内であることを見る）。
    for (name, ids, term) in [
        ("messages-invisible-only", ids.clone(), "機密語"),
        ("messages-no-visible-channels", "[]".to_string(), "です"),
    ] {
        let h: f64 = one(&db, &hits("messages", "content", &[term]))
            .await?
            .parse()?;
        let params = vec![
            now.to_string(),
            ids,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "21".into(),
            term.into(),
        ];
        queries.push((
            name.into(),
            bind(&month_sql(SearchKind::Messages, &[term]), &params),
            3. * h + 64.,
        ));
    }
    let lonely: Value =
        serde_json::from_str(&one(&db, &bind(SCOPE_SQL, &[LONELY.into(), String::new()])).await?)?;
    assert_eq!(
        lonely["ids"],
        json!([]),
        "a guest without memberships sees nothing"
    );

    let queries = Arc::new(queries);
    let metrics = Arc::new(Mutex::new(Vec::<Value>::new()));
    for concurrency in [1, 8] {
        let mut tasks = Vec::new();
        for _ in 0..concurrency {
            let (url, queries, metrics) = (url.clone(), queries.clone(), metrics.clone());
            tasks.push(tokio::spawn(async move {
                let db = client(&url).await.unwrap();
                for sample in 0..30 {
                    for (name, sql, bound) in queries.iter() {
                        let start = std::time::Instant::now();
                        let raw = one(&db, &format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}")).await.unwrap();
                        let roundtrip_ms = start.elapsed().as_secs_f64() * 1000.;
                        let value: Value = serde_json::from_str(&raw).unwrap();
                        let plan = &value[0];
                        let mut relations = std::collections::BTreeMap::new();
                        scan(&plan["Plan"], &mut relations);
                        // 対象の表と検索用インデックスは上限内。JOIN 先（channels など）はページの件数分。
                        for (relation, work) in &relations {
                            let limit = if relation.ends_with("_search_month")
                                || ["messages", "attachments", "channel_notes", "join-filter-removed", "sort-input"].contains(&relation.as_str())
                                || name.starts_with("scope")
                            {
                                *bound
                            } else {
                                64.
                            };
                            assert!(*work <= limit, "{name} {relation}: {work} > {limit}: {plan}");
                        }
                        metrics.lock().unwrap().push(json!({
                            "name": name, "concurrency": concurrency,
                            "plan": if sample == 0 { plan.clone() } else { Value::Null },
                            "relations": relations,
                            "blocks": plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0) + plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0),
                            "planning_ms": plan["Planning Time"], "execution_ms": plan["Execution Time"],
                            "roundtrip_ms": roundtrip_ms,
                        }));
                    }
                }
            }));
        }
        for task in tasks {
            task.await?;
        }
    }

    // API 全体（認証・トランザクション・月ごとの SQL・JSON）。今月で 21 件そろうもの、12 か月を見て終わるもの。
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let mut config = backend::config::Config::from_env();
    config.database_url = url.clone();
    config.dev_auth_enabled = true;
    let state = backend::state::AppState::new(backend::db::init_db(&url).await?, config);
    let app = backend::routes::create_router(state);
    let api = [
        (
            "api-messages",
            "/api/search?type=messages&q=%E3%81%A7%E3%81%99",
        ),
        (
            "api-messages-12-months",
            "/api/search?type=messages&q=%E5%AD%98%E5%9C%A8%E3%81%97%E3%81%AA%E3%81%84",
        ),
        // 既定（メッセージとファイル）。月ごとに 2 本。
        ("api-all", "/api/search?q=%E3%81%A7%E3%81%99"),
        (
            "api-all-12-months",
            "/api/search?q=%E5%AD%98%E5%9C%A8%E3%81%97%E3%81%AA%E3%81%84",
        ),
        ("api-files", "/api/search?type=files&q=pdf"),
        ("api-notes", "/api/search?type=notes&q=%E9%9A%9C%E5%AE%B3"),
    ];
    let oldest_month = {
        let m = now - 11;
        format!("{:04}-{:02}", (m - 1) / 12, (m - 1) % 12 + 1)
    };
    // まれな語は、今月の削除されていない 1 行だけに当たる（0 件の計測にならない）。
    {
        let uri = "/api/search?q=%E3%83%AC%E3%82%A2%E8%AA%9E"; // レア語
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("X-Dev-User", ACTOR)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await?;
        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await?)?;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1), "{body}");
    }
    // リクエストごとの SQL の回数・合計バッファ・DB 時間（pg_stat_statements。CI と compose で preload する）。
    // 統計は全体で 1 つなので、1 リクエストずつ reset して読む。
    db.batch_execute("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .await?;
    let stat = |filter: &str| {
        format!(
            "SELECT json_build_object('calls', COALESCE(sum(calls), 0), \
               'blocks', COALESCE(sum(shared_blks_hit + shared_blks_read), 0), \
               'db_ms', COALESCE(sum(total_exec_time), 0))::text \
             FROM pg_stat_statements WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
               AND query NOT LIKE '%pg_stat_statements%' AND ({filter})"
        )
    };
    let mut api_sql = Vec::new();
    for (name, uri) in api {
        for _ in 0..5 {
            db.batch_execute("SELECT pg_stat_statements_reset()")
                .await?;
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("X-Dev-User", ACTOR)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await?;
            assert_eq!(response.status(), 200, "{name}");
            let all: Value = serde_json::from_str(&one(&db, &stat("true")).await?)?;
            let month: Value =
                serde_json::from_str(&one(&db, &stat("query LIKE '%likequery%'")).await?)?;
            let scope: Value = serde_json::from_str(
                &one(&db, &stat("query LIKE '%visible AS MATERIALIZED%'")).await?,
            )?;
            let months = month["calls"].as_f64().unwrap();
            assert_eq!(scope["calls"].as_f64(), Some(1.), "{name}: scope SQL once");
            let expected = match name {
                // 該当なしは、ちょうど 12 か月分の SQL を流して止まる。
                "api-messages-12-months" => Some(12.),
                // よくある語は今月だけでページがそろう。
                "api-messages" => Some(1.),
                "api-all-12-months" => Some(24.),
                "api-all" => Some(2.),
                _ => None,
            };
            if let Some(expected) = expected {
                assert_eq!(months, expected, "{name}: month SQL count");
            }
            assert!((1. ..=24.).contains(&months), "{name}: {months} month SQLs");
            api_sql.push(json!({
                "name": name, "month_sql": months, "statements": all["calls"],
                "blocks": all["blocks"], "db_ms": all["db_ms"],
            }));
        }
    }
    let api_metrics = Arc::new(Mutex::new(Vec::<Value>::new()));
    for concurrency in [1, 8] {
        let mut tasks = Vec::new();
        for _ in 0..concurrency {
            let (app, metrics, oldest_month) =
                (app.clone(), api_metrics.clone(), oldest_month.clone());
            tasks.push(tokio::spawn(async move {
                for _ in 0..30 {
                    for (name, uri) in api {
                        let start = std::time::Instant::now();
                        let response = app.clone().oneshot(Request::builder().uri(uri).header("X-Dev-User", ACTOR).body(Body::empty()).unwrap()).await.unwrap();
                        assert_eq!(response.status(), 200, "{name}");
                        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
                        if name.ends_with("-12-months") {
                            assert!(body["items"].as_array().unwrap().is_empty());
                            assert!(body["next_cursor"].is_string(), "older months remain after 12");
                            // ちょうど 12 か月（今月から 11 か月前まで）を見て止まる。
                            assert_eq!(body["searched_until"], json!(oldest_month), "{body}");
                        }
                        metrics.lock().unwrap().push(json!({"name": name, "concurrency": concurrency, "roundtrip_ms": start.elapsed().as_secs_f64() * 1000.}));
                    }
                }
            }));
        }
        for task in tasks {
            task.await?;
        }
    }

    let output =
        std::env::var("SEARCH_SCALE_OUTPUT").unwrap_or(format!("/tmp/search-budget-{n}.json"));
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(&json!({
            "rows": n, "recent_rows": R, "visible_channels": visible, "queries": queries.len(),
            "query_definitions": queries.iter().map(|(name, sql, bound)| json!({"name": name, "sql": sql, "bound": bound})).collect::<Vec<_>>(),
            "samples": *metrics.lock().unwrap(), "api_samples": *api_metrics.lock().unwrap(), "api_sql": api_sql,
        }))?,
    )?;
    println!(
        "search query budget: {n} rows, {} queries, output {output}",
        queries.len()
    );
    Ok(())
}
