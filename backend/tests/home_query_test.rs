//! Guard the production Home query against materializing unrelated remote posts.
use backend::config::Config;
use serde_json::Value;
use tokio_postgres::{NoTls, SimpleQueryMessage};

// Bulk fixtures hold hundreds of advisory-lock stripes until rollback. Keep
// independent fixtures apart; concurrency scenarios still use multiple sessions.
static FIXTURE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn single_post_query_scopes_work_to_the_conversation() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL jit=off; SET LOCAL statement_timeout='10s'")
        .await?;
    let user = uuid::Uuid::new_v4().to_string();
    let actor = format!("https://{}.test/actor", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&user],
    )
    .await?;
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'remote','Remote',$1,'dummy')", &[&actor]).await?;
    tx.execute("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) VALUES($1,$2,'outgoing',$2,true)", &[&user,&actor]).await?;
    tx.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id) SELECT $1||'/posts/'||n,$1,repeat('content ',50),'public',now(),now(),CASE WHEN n IN (2,3) THEN $1||'/posts/'||(n-1) END FROM generate_series(1,20000) n", &[&actor]).await?;
    tx.execute("INSERT INTO apub_remote_recipients(post_id,user_id) SELECT id,$1 FROM apub_remote_posts WHERE actor_id=$2", &[&user,&actor]).await?;
    let other = format!("{actor}/other");
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'other','Other',$1,'dummy')", &[&other]).await?;
    tx.execute("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) VALUES($1,$2,'outgoing',$2,true)", &[&user,&other]).await?;
    tx.execute("UPDATE apub_remote_posts SET actor_id=$1 WHERE actor_id=$2 AND id NOT IN ($2||'/posts/1',$2||'/posts/2',$2||'/posts/3')", &[&other,&actor]).await?;
    let post = format!("{actor}/posts/2");
    let activity = uuid::Uuid::new_v4().to_string();
    tx.execute("INSERT INTO apub_reposts(post_id,actor_id,user_id,activity_id,activity_json,visibility) VALUES($1,'local:'||$2,$2,$3,'{}','public')", &[&post,&user,&activity]).await?;
    let repost = format!("repost:{activity}");
    let entries = serde_json::json!([post]).to_string();
    let repost_entries = serde_json::json!([repost]).to_string();
    let author_entries = serde_json::json!([
        format!("{actor}/posts/1"),
        format!("{actor}/posts/2"),
        format!("{actor}/posts/3")
    ])
    .to_string();
    for (post_id, author, window, thread, expected_rows, expected_base) in [
        (post.as_str(), "", "[]", "false", 1, 2),
        (post.as_str(), "", "[]", "true", 3, 3),
        ("", actor.as_str(), author_entries.as_str(), "false", 3, 3),
        ("", "", entries.as_str(), "false", 1, 2),
        ("", "", repost_entries.as_str(), "false", 1, 2),
    ] {
        let mut sql = backend::routes::home::timeline_query();
        // All values are fixed literals or locally generated UUIDs, never input.
        for (n, value) in [
            user.as_str(),
            "",
            "social",
            "false",
            "500",
            post_id,
            author,
            thread,
            window,
        ]
        .iter()
        .enumerate()
        .rev()
        {
            sql = sql.replace(&format!("${}", n + 1), &format!("'{value}'"));
        }
        let result = tx
            .simple_query(&format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}"))
            .await?;
        let json = result
            .iter()
            .find_map(|r| match r {
                SimpleQueryMessage::Row(row) => row.get(0),
                _ => None,
            })
            .unwrap();
        let plan: Value = serde_json::from_str(json)?;
        fn find_base(node: &Value) -> Option<&Value> {
            if node["Subplan Name"] == "CTE base" {
                return Some(node);
            }
            node["Plans"].as_array()?.iter().find_map(find_base)
        }
        let base = find_base(&plan[0]["Plan"]).expect("materialized audience-filtered base");
        assert_eq!(
            base["Actual Rows"], expected_base,
            "unrelated posts must not enter the visible base: {base}"
        );
        assert_eq!(plan[0]["Plan"]["Actual Rows"], expected_rows);
    }
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn notification_previews_are_built_only_for_the_selected_page() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL jit=off; SET LOCAL statement_timeout='15s'")
        .await?;
    let user = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&user],
    )
    .await?;
    tx.execute("INSERT INTO apub_notifications(id,user_id,actor_id,activity_id,kind) SELECT $1||'-'||n,$1,'peer',$1||'-activity-'||n,'like' FROM generate_series(1,2000) n",&[&user]).await?;
    tx.batch_execute("ANALYZE notification_events").await?;
    let sql = include_str!("../src/activitypub/notification_page.sql")
        .replace("$1", &format!("'{user}'"))
        .replace("$2", "''")
        .replace("$3", "'[]'");
    let rows = tx
        .simple_query(&format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}"))
        .await?;
    let raw = rows
        .iter()
        .find_map(|row| match row {
            SimpleQueryMessage::Row(row) => row.get(0),
            _ => None,
        })
        .unwrap();
    let plan: Value = serde_json::from_str(raw)?;
    fn page(node: &Value) -> Option<&Value> {
        if node["Subplan Name"] == "CTE notice_page" {
            return Some(node);
        }
        node["Plans"].as_array()?.iter().find_map(page)
    }
    fn bounded_notice_scan(node: &Value) {
        if node["Relation Name"] == "notification_events" {
            let read = (node["Actual Rows"].as_f64().unwrap_or(0.)
                + node["Rows Removed by Filter"].as_f64().unwrap_or(0.))
                * node["Actual Loops"].as_f64().unwrap_or(0.);
            assert!(
                read <= 102.,
                "notification history was scanned before paging: {node}"
            );
        }
        if let Some(children) = node["Plans"].as_array() {
            for child in children {
                bounded_notice_scan(child);
            }
        }
    }
    bounded_notice_scan(&plan[0]["Plan"]);
    assert_eq!(page(&plan[0]["Plan"]).unwrap()["Actual Rows"], 101);
    assert_eq!(plan[0]["Plan"]["Actual Rows"], 101);
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn timeline_seek_metadata_tracks_legacy_writers_and_late_parents() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let user = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let actor = format!("https://{user}.test/actor");
    let post = format!("{actor}/post");
    let parent = format!("{user}-parent");
    tx.execute("INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)", &[&user,&other]).await?;
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'remote','Remote',$1,'dummy')", &[&actor]).await?;
    tx.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id) VALUES($1,$2,'body','public','2026-01-01',now(),$3)", &[&post,&actor,&parent]).await?;
    // The old application supplies only these two columns.
    tx.execute(
        "INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2)",
        &[&post, &user],
    )
    .await?;
    let row=tx.query_one("SELECT actor_id,reply_to_user,post_deleted,published='2026-01-01'::timestamptz FROM apub_remote_recipients WHERE post_id=$1", &[&post]).await?;
    assert_eq!(row.get::<_, String>(0), actor);
    assert!(!row.get::<_, bool>(1));
    assert!(!row.get::<_, bool>(2));
    assert!(row.get::<_, bool>(3));
    tx.execute("INSERT INTO home_posts(id,user_id,content,visibility) VALUES($1,$2,'late parent','public')", &[&parent,&user]).await?;
    assert!(
        tx.query_one(
            "SELECT reply_to_user FROM apub_remote_recipients WHERE post_id=$1",
            &[&post]
        )
        .await?
        .get::<_, bool>(0)
    );
    tx.execute(
        "UPDATE home_posts SET user_id=$2 WHERE id=$1",
        &[&parent, &other],
    )
    .await?;
    assert!(
        !tx.query_one(
            "SELECT reply_to_user FROM apub_remote_recipients WHERE post_id=$1",
            &[&post]
        )
        .await?
        .get::<_, bool>(0)
    );
    tx.execute(
        "UPDATE home_posts SET user_id=$2 WHERE id=$1",
        &[&parent, &user],
    )
    .await?;
    tx.execute(
        "UPDATE apub_remote_posts SET published='2026-02-01',deleted_at=now() WHERE id=$1",
        &[&post],
    )
    .await?;
    let row=tx.query_one("SELECT post_deleted,published='2026-02-01'::timestamptz FROM apub_remote_recipients WHERE post_id=$1", &[&post]).await?;
    assert!(row.get::<_, bool>(0));
    assert!(row.get::<_, bool>(1));
    tx.execute(
        "UPDATE apub_remote_posts SET deleted_at=NULL,parent_id=NULL WHERE id=$1",
        &[&post],
    )
    .await?;
    let row = tx
        .query_one(
            "SELECT reply_to_user,post_deleted FROM apub_remote_recipients WHERE post_id=$1",
            &[&post],
        )
        .await?;
    assert!(!row.get::<_, bool>(0));
    assert!(!row.get::<_, bool>(1));
    let activity = format!("{actor}/announce");
    tx.execute("INSERT INTO apub_reposts(post_id,actor_id,activity_id,activity_json,visibility) VALUES($1,$2,$3,'{}','public')", &[&post,&actor,&activity]).await?;
    tx.execute(
        "INSERT INTO apub_repost_recipients(activity_id,user_id) VALUES($1,$2)",
        &[&activity, &user],
    )
    .await?;
    tx.execute(
        "UPDATE apub_reposts SET created_at='2026-03-01' WHERE activity_id=$1",
        &[&activity],
    )
    .await?;
    let row=tx.query_one("SELECT actor_id,created_at='2026-03-01'::timestamptz FROM apub_repost_recipients WHERE activity_id=$1", &[&activity]).await?;
    assert_eq!(row.get::<_, String>(0), actor);
    assert!(row.get::<_, bool>(1));
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn concurrent_parent_and_receipt_insertion_keeps_reply_seek_eligible() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let url = Config::from_env().database_url;
    let (mut a, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let (mut b, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let user = uuid::Uuid::new_v4().to_string();
    let actor = format!("https://{user}.test/actor");
    let post = format!("{actor}/post");
    let parent = format!("{user}-parent");
    a.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&user],
    )
    .await?;
    a.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'remote','Remote',$1,'dummy')", &[&actor]).await?;
    a.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id) VALUES($1,$2,'body','public',now(),now(),$3)", &[&post,&actor,&parent]).await?;
    let tx = a.transaction().await?;
    tx.execute(
        "INSERT INTO home_posts(id,user_id,content,visibility) VALUES($1,$2,'parent','public')",
        &[&parent, &user],
    )
    .await?;
    let post2 = post.clone();
    let user2 = user.clone();
    let mut insertion = tokio::spawn(async move {
        let tx = b.transaction().await?;
        tx.execute("SET LOCAL statement_timeout='5s'", &[]).await?;
        tx.execute(
            "INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2)",
            &[&post2, &user2],
        )
        .await?;
        tx.commit().await
    });
    // Receipt must wait for the concurrent parent's commit, then see its owner.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut insertion)
            .await
            .is_err()
    );
    tx.commit().await?;
    insertion.await??;
    let eligible = a
        .query_one(
            "SELECT reply_to_user FROM apub_remote_recipients WHERE post_id=$1",
            &[&post],
        )
        .await?
        .get::<_, bool>(0);
    a.execute("DELETE FROM apub_remote_posts WHERE id=$1", &[&post])
        .await?;
    a.execute("DELETE FROM apub_actors WHERE id=$1", &[&actor])
        .await?;
    a.execute("DELETE FROM users WHERE id=$1", &[&user]).await?;
    assert!(
        eligible,
        "receipt missed a concurrently inserted local parent"
    );
    Ok(())
}

#[tokio::test]
async fn seek_skips_private_history_and_preserves_repost_ties() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL jit=off; SET LOCAL statement_timeout='15s'")
        .await?;
    let owner = uuid::Uuid::new_v4().to_string();
    let viewer = uuid::Uuid::new_v4().to_string();
    tx.execute("INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)", &[&owner,&viewer]).await?;
    tx.execute("INSERT INTO home_posts(id,user_id,content,created_at,visibility) SELECT $1||'-hidden-'||n,$1,'private',now(),'followers' FROM generate_series(1,20000)n", &[&owner]).await?;
    tx.execute("INSERT INTO home_posts(id,user_id,content,created_at,visibility) SELECT $1||'-visible-'||n,$1,'public','2026-01-01','public' FROM generate_series(1,10)n", &[&owner]).await?;
    tx.batch_execute("ANALYZE home_posts").await?;
    fn sql(args: &[&str]) -> String {
        let mut s = backend::routes::home::timeline_page_query();
        for (n, v) in args.iter().enumerate().rev() {
            s = s.replace(
                &format!("${}", n + 1),
                &format!("'{}'", v.replace('\'', "''")),
            );
        }
        s
    }
    let q = sql(&[
        &viewer, "", "social", "false", "50", "", &owner, "false", "[]",
    ]);
    let result = tx
        .simple_query(&format!("EXPLAIN (ANALYZE,FORMAT JSON) {q}"))
        .await?;
    let p: Value = serde_json::from_str(
        result
            .iter()
            .find_map(|r| match r {
                SimpleQueryMessage::Row(r) => r.get(0),
                _ => None,
            })
            .unwrap(),
    )?;
    assert_eq!(p[0]["Plan"]["Actual Rows"], 10);
    fn check(n: &Value) {
        if n["Relation Name"] == "home_posts" {
            let read = (n["Actual Rows"].as_f64().unwrap_or(0.)
                + n["Rows Removed by Filter"].as_f64().unwrap_or(0.))
                * n["Actual Loops"].as_f64().unwrap_or(0.);
            assert!(read <= 100., "scanned private history: {n}");
        }
        if let Some(children) = n["Plans"].as_array() {
            for c in children {
                check(c);
            }
        }
    }
    check(&p[0]["Plan"]);
    // Own the public rows only; a Home page then has 10 originals + 3 reposts
    // with exactly equal timestamps and PostgreSQL's text ordering.
    tx.execute(
        "UPDATE home_posts SET user_id=$2 WHERE user_id=$1 AND visibility='public'",
        &[&owner, &viewer],
    )
    .await?;
    for (n, suffix) in [(1, "Z"), (2, "z"), (3, "z1")] {
        let post = format!("{owner}-visible-{n}");
        let activity = format!("{viewer}-{suffix}");
        tx.execute("INSERT INTO apub_reposts(post_id,actor_id,user_id,activity_id,activity_json,visibility,created_at) VALUES($1,'local:'||$2,$2,$3,'{}','public','2026-01-01')", &[&post,&viewer,&activity]).await?;
    }
    let expected:Vec<String>=tx.query("SELECT id FROM (SELECT id FROM home_posts WHERE user_id=$1 UNION ALL SELECT 'repost:'||activity_id FROM apub_reposts WHERE user_id=$1) keys ORDER BY id DESC", &[&viewer]).await?.into_iter().map(|r|r.get(0)).collect();
    let mut actual = vec![];
    let mut before = String::new();
    loop {
        let q = sql(&[
            &viewer, &before, "home", "false", "3", "", "", "false", "[]",
        ]);
        let result = tx.simple_query(&q).await?;
        let keys: Vec<String> = result
            .iter()
            .filter_map(|r| match r {
                SimpleQueryMessage::Row(r) => r.get(0),
                _ => None,
            })
            .map(|s| {
                serde_json::from_str::<Value>(s).unwrap()["entry_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        if keys.is_empty() {
            break;
        }
        before = keys.last().unwrap().clone();
        actual.extend(keys);
        assert!(actual.len() <= 13, "cursor did not advance");
    }
    assert_eq!(actual, expected);
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn selected_reposts_use_primary_keys_and_reactions_are_aggregated_once() -> anyhow::Result<()>
{
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL jit=off; SET LOCAL statement_timeout='30s'")
        .await?;
    let user = uuid::Uuid::new_v4().to_string();
    let post = format!("{user}-post");
    let activity = format!("{user}-announce");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&user],
    )
    .await?;
    tx.execute(
        "INSERT INTO home_posts(id,user_id,content,visibility) VALUES($1,$2,'post','public')",
        &[&post, &user],
    )
    .await?;
    tx.execute("INSERT INTO apub_reposts(post_id,actor_id,user_id,activity_id,activity_json,visibility) SELECT $1||'-unrelated-'||n,'local:'||$1,$1,$1||'-activity-'||n,'{}','public' FROM generate_series(1,30000)n", &[&user]).await?;
    tx.execute("INSERT INTO apub_reposts(post_id,actor_id,user_id,activity_id,activity_json,visibility) VALUES($1,'local:'||$2,$2,$3,'{}','public')", &[&post,&user,&activity]).await?;
    let actor = format!("https://{user}.test/actor/1");
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key,blocked) VALUES($1,$1,$1,'blocked','Blocked',$1,'dummy',true)", &[&actor]).await?;
    tx.execute("INSERT INTO apub_reactions(post_id,actor_id,activity_id,activity_json,reaction) SELECT $1,$2||n,$1||'-reaction-'||n,'{}',CASE WHEN n%2=0 THEN '❤️' ELSE '😀' END FROM generate_series(1,5000)n", &[&post,&format!("https://{user}.test/actor/")]).await?;
    tx.execute("INSERT INTO apub_reactions(post_id,actor_id,user_id,activity_id,activity_json,reaction) VALUES($1,'local:'||$2,$2,$1||'-mine','{}','👍')", &[&post,&user]).await?;
    let remote_actor = format!("https://{user}.test/unfollowed");
    let remote_post = format!("{remote_actor}/post");
    let remote_activity = format!("{user}-remote-announce");
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'remote','Remote',$1,'dummy')", &[&remote_actor]).await?;
    tx.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at) VALUES($1,$2,'only visible by repost','public',now(),now())", &[&remote_post,&remote_actor]).await?;
    tx.execute("INSERT INTO apub_reposts(post_id,actor_id,user_id,activity_id,activity_json,visibility) VALUES($1,'local:'||$2,$2,$3,'{}','public')", &[&remote_post,&user,&remote_activity]).await?;
    tx.batch_execute("ANALYZE apub_reposts; ANALYZE apub_reactions; ANALYZE apub_actors")
        .await?;
    for entry in [
        &post,
        &format!("repost:{activity}"),
        &format!("repost:{remote_activity}"),
    ] {
        let window = serde_json::json!([entry]).to_string();
        let mut sql = backend::routes::home::timeline_query();
        for (n, v) in [&user, "", "social", "false", "50", "", "", "false", &window]
            .iter()
            .enumerate()
            .rev()
        {
            sql = sql.replace(
                &format!("${}", n + 1),
                &format!("'{}'", v.replace('\'', "''")),
            );
        }
        let rows = tx
            .simple_query(&format!("EXPLAIN (ANALYZE,FORMAT JSON) {sql}"))
            .await?;
        let p: Value = serde_json::from_str(
            rows.iter()
                .find_map(|r| match r {
                    SimpleQueryMessage::Row(r) => r.get(0),
                    _ => None,
                })
                .unwrap(),
        )?;
        let mut repost_reads = 0.;
        let mut reaction_reads = 0.;
        fn scan(n: &Value, reposts: &mut f64, reactions: &mut f64) {
            let read = (n["Actual Rows"].as_f64().unwrap_or(0.)
                + n["Rows Removed by Filter"].as_f64().unwrap_or(0.))
                * n["Actual Loops"].as_f64().unwrap_or(0.);
            match n["Relation Name"].as_str() {
                Some("apub_reposts") => {
                    *reposts += read;
                    if read > 5. {
                        eprintln!("unexpected repost scan: {n}");
                    }
                }
                Some("apub_reactions") => *reactions += read,
                _ => (),
            }
            if let Some(children) = n["Plans"].as_array() {
                for c in children {
                    scan(c, reposts, reactions);
                }
            }
        }
        scan(&p[0]["Plan"], &mut repost_reads, &mut reaction_reads);
        assert!(
            repost_reads <= 10.,
            "unrelated repost history was scanned: {repost_reads}"
        );
        assert!(
            reaction_reads <= 5001.,
            "reaction stream was scanned repeatedly: {reaction_reads}"
        );
        let result = tx.simple_query(&sql).await?;
        let rows: Vec<Value> = result
            .iter()
            .filter_map(|r| match r {
                SimpleQueryMessage::Row(r) => r.get(0),
                _ => None,
            })
            .map(|s| backend::routes::home::timeline_post(serde_json::from_str(s).unwrap()))
            .collect();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["entry_id"], entry.as_str());
        if entry == &format!("repost:{remote_activity}") {
            assert_eq!(row["id"], remote_post);
            assert_eq!(row["likes"], 0);
            assert_eq!(row["liked"], false);
            assert_eq!(row["reactions"], serde_json::json!([]));
            continue;
        }
        assert_eq!(row["likes"], 5000);
        assert_eq!(row["liked"], true);
        assert_eq!(row["my_reaction"], "👍");
        let groups = row["reactions"].as_array().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups.iter().filter(|g| g["selected"] == true).count(), 1);
        assert_eq!(
            groups
                .iter()
                .map(|g| g["count"].as_i64().unwrap())
                .sum::<i64>(),
            5000
        );
    }
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn activitypub_foreign_keys_have_full_leading_indexes() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    for (table, column) in [
        ("apub_remote_posts", "actor_id"),
        ("apub_remote_recipients", "user_id"),
        ("apub_follows", "actor_id"),
        ("apub_deliveries", "actor_id"),
    ] {
        let row=client.query_one("SELECT EXISTS(SELECT 1 FROM pg_index i JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=i.indkey[0] WHERE i.indrelid=$1::text::regclass AND a.attname=$2 AND i.indisvalid AND i.indisready AND i.indpred IS NULL)", &[&table,&column]).await?;
        assert!(
            row.get::<_, bool>(0),
            "foreign-key checks on {table}.{column} need a leading full index"
        );
    }
    Ok(())
}

#[tokio::test]
async fn reply_counters_follow_moves_deletes_and_user_cascades() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let user = uuid::Uuid::new_v4().to_string();
    let actor = format!("https://{user}.test/actor");
    let post = format!("{actor}/post");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&user],
    )
    .await?;
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'remote','Remote',$1,'dummy')", &[&actor]).await?;
    tx.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id) VALUES($1,$2,'reply','public',now(),now(),'parent-one')", &[&post,&actor]).await?;
    tx.execute(
        "INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
        &[&post, &user],
    )
    .await?;
    tx.execute(
        "INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
        &[&post, &user],
    )
    .await?;
    assert_eq!(
        tx.query_one(
            "SELECT n FROM apub_reply_counts WHERE user_id=$1 AND parent_id='parent-one'",
            &[&user]
        )
        .await?
        .get::<_, i64>(0),
        1
    );
    tx.execute(
        "UPDATE apub_remote_posts SET parent_id='parent-two' WHERE id=$1",
        &[&post],
    )
    .await?;
    assert_eq!(
        tx.query_one(
            "SELECT n FROM apub_reply_counts WHERE user_id=$1 AND parent_id='parent-one'",
            &[&user]
        )
        .await?
        .get::<_, i64>(0),
        0
    );
    tx.execute(
        "UPDATE apub_remote_posts SET deleted_at=now() WHERE id=$1",
        &[&post],
    )
    .await?;
    assert_eq!(
        tx.query_one(
            "SELECT n FROM apub_reply_counts WHERE user_id=$1 AND parent_id='parent-two'",
            &[&user]
        )
        .await?
        .get::<_, i64>(0),
        0
    );
    tx.execute(
        "UPDATE apub_remote_posts SET deleted_at=NULL WHERE id=$1",
        &[&post],
    )
    .await?;
    assert_eq!(
        tx.query_one(
            "SELECT n FROM apub_reply_counts WHERE user_id=$1 AND parent_id='parent-two'",
            &[&user]
        )
        .await?
        .get::<_, i64>(0),
        1
    );
    tx.execute("DELETE FROM users WHERE id=$1", &[&user])
        .await?;
    assert_eq!(
        tx.query_one(
            "SELECT count(*) FROM apub_reply_counts WHERE user_id=$1",
            &[&user]
        )
        .await?
        .get::<_, i64>(0),
        0
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn counter_backfill_serializes_with_concurrent_receipt_delta() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let url = Config::from_env().database_url;
    let (mut a, c) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(c);
    let (b, c) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(c);
    let user = uuid::Uuid::new_v4().to_string();
    let actor = format!("https://{user}.test/actor");
    let post = format!("{actor}/post");
    a.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&user],
    )
    .await?;
    a.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'actor','Actor',$1,'dummy')", &[&actor]).await?;
    a.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id) VALUES($1,$2,'reply','public',now(),now(),'backfill-parent')", &[&post,&actor]).await?;
    a.execute(
        "INSERT INTO apub_reply_counts(user_id,parent_id,actor_id) VALUES($1,'backfill-parent',$2)",
        &[&user, &actor],
    )
    .await?;
    let tx = a.transaction().await?;
    tx.query_one(
        "SELECT n FROM apub_reply_counts WHERE user_id=$1 FOR UPDATE",
        &[&user],
    )
    .await?;
    let u = user.clone();
    let p = post.clone();
    let mut writer = tokio::spawn(async move {
        b.execute(
            "INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2)",
            &[&p, &u],
        )
        .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut writer)
            .await
            .is_err()
    );
    tx.execute("UPDATE apub_reply_counts c SET n=(SELECT count(*) FROM apub_remote_recipients r WHERE r.user_id=c.user_id AND r.parent_id=c.parent_id AND r.actor_id=c.actor_id AND NOT r.post_deleted) WHERE c.user_id=$1", &[&user]).await?;
    tx.commit().await?;
    writer.await??;
    let n = a
        .query_one("SELECT n FROM apub_reply_counts WHERE user_id=$1", &[&user])
        .await?
        .get::<_, i64>(0);
    a.execute("DELETE FROM users WHERE id=$1", &[&user]).await?;
    a.execute("DELETE FROM apub_remote_posts WHERE id=$1", &[&post])
        .await?;
    a.execute("DELETE FROM apub_actors WHERE id=$1", &[&actor])
        .await?;
    assert_eq!(
        n, 1,
        "backfill must not lose or double count the waiting delta"
    );
    Ok(())
}

#[tokio::test]
async fn moving_a_remote_reply_waits_for_both_parent_stripes() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let url = Config::from_env().database_url;
    let (mut owner, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let (mut mover, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let id = uuid::Uuid::new_v4().to_string();
    let other = format!("{id}-other");
    let x = format!("{id}-x");
    let mut y = format!("{id}-y");
    let child = format!("https://{id}.test/reply");
    let stripe: i32 = owner
        .query_one("SELECT (hashtextextended($1,0)&1023)::integer", &[&x])
        .await?
        .get(0);
    while owner
        .query_one("SELECT (hashtextextended($1,0)&1023)::integer", &[&y])
        .await?
        .get::<_, i32>(0)
        == stripe
    {
        y.push('y');
    }
    owner.execute("INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)",&[&id,&other]).await?;
    owner.execute("INSERT INTO home_posts(id,user_id,content,visibility) VALUES($1,$3,'x','public'),($2,$3,'y','public')",&[&x,&y,&id]).await?;
    owner.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'actor','Actor',$1,'dummy')",&[&child]).await?;
    owner.execute("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id) VALUES($1,$1,'reply','public',now(),now(),$2)",&[&child,&x]).await?;
    owner
        .execute(
            "INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2)",
            &[&child, &id],
        )
        .await?;
    let pid: i32 = mover
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let tx = owner.transaction().await?;
    tx.execute("SELECT pg_advisory_xact_lock(20880,$1)", &[&stripe])
        .await?;
    let target = y.clone();
    let post = child.clone();
    let task = tokio::spawn(async move {
        let tx = mover.transaction().await?;
        tx.execute(
            "UPDATE apub_remote_posts SET parent_id=$1 WHERE id=$2",
            &[&target, &post],
        )
        .await?;
        tx.commit().await?;
        Ok::<_, anyhow::Error>(())
    });
    let mut advisory = false;
    for _ in 0..100 {
        tx.batch_execute("SELECT pg_stat_clear_snapshot()").await?;
        let event: Option<String> = tx
            .query_one(
                "SELECT wait_event FROM pg_stat_activity WHERE pid=$1",
                &[&pid],
            )
            .await?
            .get(0);
        if event.as_deref() == Some("advisory") {
            advisory = true;
            break;
        }
        if task.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        advisory,
        "parent move must wait on the old-parent stripe, before receipt row locks"
    );
    tx.execute(
        "UPDATE home_posts SET user_id=$1 WHERE id=$2",
        &[&other, &x],
    )
    .await?;
    tx.commit().await?;
    task.await??;
    let row=owner.query_one("SELECT parent_id,reply_to_user FROM apub_remote_recipients WHERE post_id=$1 AND user_id=$2",&[&child,&id]).await?;
    assert_eq!(row.get::<_, String>(0), y);
    assert!(row.get::<_, bool>(1));
    owner
        .execute("DELETE FROM apub_remote_posts WHERE id=$1", &[&child])
        .await?;
    owner
        .execute("DELETE FROM home_posts WHERE id IN($1,$2)", &[&x, &y])
        .await?;
    owner
        .execute("DELETE FROM apub_actors WHERE id=$1", &[&child])
        .await?;
    owner
        .execute("DELETE FROM users WHERE id IN($1,$2)", &[&id, &other])
        .await?;
    Ok(())
}

#[tokio::test]
async fn reaction_counter_recount_serializes_with_a_concurrent_delta() -> anyhow::Result<()> {
    let _fixture = FIXTURE_SERIAL.lock().await;
    let url = Config::from_env().database_url;
    let (mut owner, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let (writer, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let post = format!("https://{}.test/counter", uuid::Uuid::new_v4());
    owner.execute("INSERT INTO apub_reactions(post_id,actor_id,activity_id,activity_json) VALUES($1,'actor-one',$1||'/one','{}')",&[&post]).await?;
    let tx = owner.transaction().await?;
    tx.query_one(
        "SELECT n FROM home_reaction_totals WHERE post_id=$1 FOR UPDATE",
        &[&post],
    )
    .await?;
    let target = post.clone();
    let task = tokio::spawn(async move {
        writer.execute("INSERT INTO apub_reactions(post_id,actor_id,activity_id,activity_json) VALUES($1,'actor-two',$1||'/two','{}')",&[&target]).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        !task.is_finished(),
        "source writer must wait for its delta counter"
    );
    tx.execute("UPDATE home_reaction_totals SET n=(SELECT count(*) FROM apub_reactions WHERE post_id=$1) WHERE post_id=$1",&[&post]).await?;
    tx.commit().await?;
    task.await??;
    assert_eq!(
        owner
            .query_one(
                "SELECT n FROM home_reaction_totals WHERE post_id=$1",
                &[&post]
            )
            .await?
            .get::<_, i64>(0),
        2
    );
    owner
        .execute("DELETE FROM apub_reactions WHERE post_id=$1", &[&post])
        .await?;
    assert_eq!(
        owner
            .query_one(
                "SELECT n FROM home_reaction_totals WHERE post_id=$1",
                &[&post]
            )
            .await?
            .get::<_, i64>(0),
        0
    );
    owner
        .execute(
            "DELETE FROM home_reaction_totals WHERE post_id=$1",
            &[&post],
        )
        .await?;
    Ok(())
}
