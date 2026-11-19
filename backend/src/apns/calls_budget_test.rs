use super::CALL_PAYLOAD_SQL;
use crate::call_invites::PENDING_INVITATION_SQL;
use futures_util::{FutureExt, future::try_join_all};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};

#[path = "calls_api_budget_test.rs"]
mod api;

async fn connect(url: &str, schema: &str) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(connection);
    client
        .batch_execute(&format!("SET search_path TO {schema}; SET jit=off"))
        .await?;
    Ok(client)
}

fn scanned(plan: &Value, counts: &mut BTreeMap<String, f64>) {
    if let Some(name) = plan["Relation Name"].as_str() {
        let rows = plan["Actual Rows"].as_f64().unwrap_or(0.0)
            + plan["Rows Removed by Filter"].as_f64().unwrap_or(0.0)
            + plan["Rows Removed by Index Recheck"]
                .as_f64()
                .unwrap_or(0.0);
        *counts.entry(name.to_owned()).or_default() +=
            rows * plan["Actual Loops"].as_f64().unwrap_or(0.0);
    }
    assert_ne!(plan["Sort Space Type"].as_str(), Some("Disk"));
    if let Some(children) = plan["Plans"].as_array() {
        for child in children {
            scanned(child, counts);
        }
    }
}

async fn measure(url: String, schema: String, workers: usize) -> anyhow::Result<Vec<Value>> {
    let mut tasks = Vec::new();
    for worker in 0..workers {
        let url = url.clone();
        let schema = schema.clone();
        tasks.push(async move {
            let at = Instant::now();
            let client = connect(&url, &schema).await?;
            let connection_ms = at.elapsed().as_secs_f64() * 1000.0;
            let mut samples = Vec::new();
            for plan_mode in ["force_custom_plan", "force_generic_plan"] {
                client.batch_execute(&format!("SET plan_cache_mode={plan_mode}; PREPARE incoming(text,text,text) AS {CALL_PAYLOAD_SQL}; PREPARE pending(text,text) AS {PENDING_INVITATION_SQL}; PREPARE devices(text) AS SELECT * FROM apns_subscriptions WHERE user_id=$1")).await?;
                for (name, execute) in [
                    ("着信", "incoming(md5('1'),'s1','secret')"),
                    ("解除情報不一致", "incoming(md5('1'),'s1','wrong')"),
                    ("存在しない招待", "incoming('00000000-0000-0000-0000-000000000000','s1','secret')"),
                    ("全20登録", "devices('u1')"),
                    ("登録なし", "devices('absent')"),
                    ("招待状態", "pending(md5('1'),'u1')"),
                    ("別人の招待", "pending(md5('1'),'u2')"),
                ] {
                    for attempt in 0..30 {
                        let at = Instant::now();
                        let result = client.simple_query(&format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) EXECUTE {execute}")).await?;
                        let elapsed = at.elapsed().as_secs_f64() * 1000.0;
                        let value: Value = result.iter().find_map(|message| {
                            if let SimpleQueryMessage::Row(row) = message { row.get(0) } else { None }
                        }).map(serde_json::from_str).transpose()?.ok_or_else(|| anyhow::anyhow!("実行計画がありません"))?;
                        let plan = &value[0]["Plan"];
                        let mut counts = BTreeMap::new();
                        scanned(plan, &mut counts);
                        for (relation, rows) in &counts {
                            let limit = if name == "全20登録" { 20.0 } else if relation == "users" { 2.0 } else { 1.0 };
                            anyhow::ensure!(*rows <= limit, "{name}: {relation}の走査上限を超過: {rows}");
                        }
                        let expected = match name { "着信" | "招待状態" => 1.0, "全20登録" => 20.0, _ => 0.0 };
                        assert_eq!(plan["Actual Rows"].as_f64(), Some(expected));
                        samples.push(json!({"条件":name,"接続数":workers,"接続":worker,"回":attempt,"計画方式":plan_mode,
                            "接続ms":connection_ms,"往復ms":elapsed,"計画ms":value[0]["Planning Time"],"実行ms":value[0]["Execution Time"],
                            "走査":counts,"ブロック":plan["Shared Hit Blocks"].as_u64().unwrap_or(0)+plan["Shared Read Blocks"].as_u64().unwrap_or(0),
                            "初回計画":if attempt==0 {value.clone()} else {Value::Null}}));
                    }
                }
                client.batch_execute("DEALLOCATE incoming; DEALLOCATE pending; DEALLOCATE devices").await?;
            }
            Ok::<_, anyhow::Error>(samples)
        });
    }
    let mut samples = Vec::new();
    for result in try_join_all(tasks).await? {
        samples.extend(result);
    }
    Ok(samples)
}

#[tokio::test]
#[ignore = "専用ローカルDBで30万・100万件の探索上限を測定"]
async fn incoming_call_query_budget() -> anyhow::Result<()> {
    let url = std::env::var("ZOO_CALL_BUDGET_DATABASE_URL")?;
    let schema = format!("call_budget_{}", uuid::Uuid::new_v4().simple());
    let admin = connect(&url, "public").await?;
    admin
        .batch_execute(&format!("CREATE SCHEMA {schema}"))
        .await?;
    let result = std::panic::AssertUnwindSafe(run_budget(&url, &schema))
        .catch_unwind()
        .await;
    // 途中のSQL・測定・保存・assertの失敗でも、この実行専用のschemaだけを回収する。
    let cleanup = admin
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await;
    if let Err(error) = &cleanup {
        eprintln!("測定用schemaの回収に失敗しました: {schema}: {error}");
    }
    match result {
        Ok(result) => result?,
        Err(panic) => std::panic::resume_unwind(panic),
    }
    cleanup?;
    Ok(())
}

async fn run_budget(url: &str, schema: &str) -> anyhow::Result<()> {
    let machine = std::env::var("ZOO_CALL_BUDGET_MACHINE")?;
    let client = connect(url, schema).await?;
    let mut results = Vec::new();
    for total in [300_000, 1_000_000] {
        // 本番と同じ検索列・型・一意性・索引を再現する。測定は読み取りだけで副作用はない。
        client.batch_execute(&format!("
            CREATE TABLE users(id text PRIMARY KEY,user_name text,display_name text,is_deactivated bool,is_guest bool,is_bot bool DEFAULT false,created_at timestamp DEFAULT now(),updated_at timestamp);
            CREATE TABLE user_identities(id text PRIMARY KEY,user_id text,iss text,sub text,created_at timestamp DEFAULT now(),updated_at timestamp,UNIQUE(iss,sub));
            CREATE TABLE channels(id text PRIMARY KEY,deleted_at timestamp,archive_at timestamp,encrypted bool,is_private bool,is_dm bool);
            CREATE TABLE apns_subscriptions(id text PRIMARY KEY,user_id text,device_token text,environment text,revocation_hash text,created_at timestamp);
            CREATE INDEX idx_apns_subscriptions_user_id ON apns_subscriptions(user_id);
            CREATE TABLE call_invites(id uuid PRIMARY KEY,channel_id text,inviter_id text,invitee_id text,status text,expires_at timestamptz);
            CREATE TABLE channel_user(user_id text,channel_id text,PRIMARY KEY(user_id,channel_id));
            CREATE TABLE user_event_settings(user_id text PRIMARY KEY,dnd_until timestamptz);
            INSERT INTO users(id,user_name,display_name,is_deactivated,is_guest) SELECT 'u'||n,'利用者','発信者',false,false FROM generate_series(1,{total}) n;
            INSERT INTO user_identities(id,user_id,iss,sub) SELECT 'i'||n,'u'||n,'https://incoming.example.invalid','u'||n FROM generate_series(1,{total}) n;
            INSERT INTO channels SELECT 'c'||n,NULL,NULL,false,true,true FROM generate_series(1,{total}) n;
            INSERT INTO apns_subscriptions SELECT 's'||n,CASE WHEN n<=20 THEN 'u1' ELSE 'u'||(n-19) END,'token'||n,'sandbox','secret',now() FROM generate_series(1,{total}) n;
            INSERT INTO call_invites SELECT md5(n::text)::uuid,'c'||n,'u2',CASE WHEN n=1 THEN 'u1' ELSE 'u2' END,CASE WHEN n=1 THEN 'pending' ELSE 'cancelled' END,now()+interval '1 hour' FROM generate_series(1,{total}) n;
            INSERT INTO channel_user SELECT 'u'||n,'c'||n FROM generate_series(1,{total}) n;
            INSERT INTO user_event_settings SELECT 'u'||n,NULL FROM generate_series(1,{total}) n;
            ANALYZE;" )).await?;
        let mut samples = measure(url.to_owned(), schema.to_owned(), 1).await?;
        samples.extend(measure(url.to_owned(), schema.to_owned(), 8).await?);
        let api = api::measure(url, schema).await?;
        results.push(json!({"件数":total,"測定":samples,"API":api}));
        client.batch_execute("DROP TABLE user_event_settings,channel_user,call_invites,apns_subscriptions,channels,user_identities,users").await?;
    }
    for (small, large) in results[0]["測定"]
        .as_array()
        .unwrap()
        .iter()
        .zip(results[1]["測定"].as_array().unwrap())
    {
        anyhow::ensure!(
            large["ブロック"].as_u64().unwrap() <= 2 * small["ブロック"].as_u64().unwrap() + 16,
            "履歴量に比例して論理ブロックが増加しました"
        );
    }
    let version = client
        .query_one("SHOW server_version", &[])
        .await?
        .get::<_, String>(0);
    let output = std::env::var("ZOO_CALL_BUDGET_OUTPUT")?;
    let settings: String = client.query_one("SELECT json_object_agg(name,json_build_object('値',setting,'単位',unit))::text FROM pg_settings WHERE name IN ('shared_buffers','work_mem','effective_cache_size','max_connections','random_page_cost','effective_io_concurrency','jit','default_statistics_target','max_parallel_workers_per_gather','plan_cache_mode')", &[]).await?.get(0);
    std::fs::write(
        output,
        serde_json::to_vec(
            &json!({"PostgreSQL":version,"機器":machine,"設定":serde_json::from_str::<Value>(&settings)?,
            "統計":"各規模のfixture投入後にANALYZE", "キャッシュ":"OS・DBキャッシュは強制破棄せず、初回も計測に含める", "結果":results}),
        )?,
    )?;
    Ok(())
}
