//! Guard the production Home query against materializing unrelated remote posts.
use backend::config::Config;
use serde_json::Value;
use tokio_postgres::{NoTls, SimpleQueryMessage};

#[tokio::test]
async fn single_post_query_scopes_work_to_the_conversation() -> anyhow::Result<()> {
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
    let post = format!("{actor}/posts/2");
    for thread in ["false", "true"] {
        let mut sql = include_str!("../src/routes/home_timeline.sql")
            .replace("/* post_scope */", "JOIN thread_ids scope ON scope.id=p.id");
        // All values are fixed literals or locally generated UUIDs, never input.
        for (n, value) in [&user, "", "home", "false", "500", &post, "", thread, "[]"]
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
            base["Actual Rows"], 3,
            "unrelated posts must not enter the visible base: {base}"
        );
        assert_eq!(
            plan[0]["Plan"]["Actual Rows"],
            if thread == "true" { 3 } else { 1 }
        );
    }
    tx.rollback().await?;
    Ok(())
}
