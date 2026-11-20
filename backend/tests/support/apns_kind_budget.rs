use super::{REGISTER, REVOKE, post, registration};
use backend::{apns::digest, config::Config, db::init_db, routes::create_router, state::AppState};
use futures_util::future::try_join_all;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};

const MIGRATION: &str =
    include_str!("../../../infra/db/migrations/20261129010003_scope_apns_registration_by_kind.sql");

pub fn migration_statements() -> (&'static str, &'static str) {
    // CONCURRENTLYとDOブロックを別の文として実行し、DO内部のセミコロンでは分割しない。
    MIGRATION.split_at(
        MIGRATION
            .find("DO $$")
            .expect("制約切り替えのDO文が必要です"),
    )
}

async fn connect(url: &str) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(connection);
    client.batch_execute("SET jit=off").await?;
    Ok(client)
}

fn milliseconds(at: Instant) -> f64 {
    at.elapsed().as_secs_f64() * 1000.0
}

fn scanned(plan: &Value, counts: &mut BTreeMap<String, f64>) {
    if let Some(name) = plan["Relation Name"].as_str() {
        // ModifyTableと子の走査ノードを重ねて数えない。
        if plan["Node Type"] != "ModifyTable" {
            let rows = plan["Actual Rows"].as_f64().unwrap_or(0.0)
                + plan["Rows Removed by Filter"].as_f64().unwrap_or(0.0)
                + plan["Rows Removed by Index Recheck"]
                    .as_f64()
                    .unwrap_or(0.0);
            *counts.entry(name.into()).or_default() +=
                rows * plan["Actual Loops"].as_f64().unwrap_or(0.0);
        }
    }
    assert_ne!(plan["Sort Space Type"].as_str(), Some("Disk"));
    if let Some(children) = plan["Plans"].as_array() {
        for child in children {
            scanned(child, counts);
        }
    }
}

async fn sql_measure(url: &str, workers: usize) -> anyhow::Result<Vec<Value>> {
    let tasks = (0..workers).map(|worker| async move {
        let at = Instant::now();
        let client = connect(url).await?;
        let connection_ms = milliseconds(at);
        let mut samples = Vec::new();
        let id = digest(&format!("sandbox:{}", digest(&(worker + 1).to_string())));
        let token = digest(&format!("追加:{worker}"));
        let insert_id = digest(&format!("sandbox:{token}"));
        for mode in ["force_custom_plan", "force_generic_plan"] {
            let at = Instant::now();
            client.batch_execute(&format!("SET plan_cache_mode={mode};
                PREPARE by_id(text) AS SELECT * FROM apns_subscriptions WHERE id=$1;
                PREPARE by_user(text) AS SELECT * FROM apns_subscriptions WHERE user_id=$1;
                PREPARE insert_one(text,text) AS INSERT INTO apns_subscriptions(id,user_id,device_token,environment,revocation_hash,created_at) VALUES($1,'budget-u1',$2,'sandbox','検証用',now());
                PREPARE update_one(text) AS UPDATE apns_subscriptions SET created_at=clock_timestamp() WHERE id=$1;
                PREPARE delete_one(text) AS DELETE FROM apns_subscriptions WHERE id=$1 AND revocation_hash='検証用' RETURNING id;")).await?;
            let preparation_ms = milliseconds(at);
            for (name, execute, limit) in [
                ("登録照会", format!("by_id('{id}')"), 1.0),
                ("空の照会", "by_id('absent')".into(), 0.0),
                ("上限20件", "by_user('budget-u1')".into(), 20.0),
                ("追加", format!("insert_one('{insert_id}','{token}')"), 1.0),
                ("更新", format!("update_one('{id}')"), 1.0),
                ("解除", format!("delete_one('{id}')"), 1.0),
            ] {
                for attempt in 0..30 {
                    client.batch_execute("BEGIN").await?;
                    let at = Instant::now();
                    let result = client.simple_query(&format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) EXECUTE {execute}")).await?;
                    let elapsed = milliseconds(at);
                    client.batch_execute("ROLLBACK").await?;
                    let plan: Value = result.iter().find_map(|m| if let SimpleQueryMessage::Row(row)=m {row.get(0)} else {None})
                        .map(serde_json::from_str).transpose()?.ok_or_else(||anyhow::anyhow!("実行計画がありません"))?;
                    let root=&plan[0]["Plan"];
                    let mut counts=BTreeMap::new();
                    scanned(root,&mut counts);
                    for count in counts.values() { anyhow::ensure!(*count <= limit,"{name}: 探索上限を超過: {count}"); }
                    if name == "上限20件" { assert_eq!(counts.get("apns_subscriptions"),Some(&20.0)); }
                    if name == "登録照会" || name == "更新" || name == "解除" { assert_eq!(counts.get("apns_subscriptions"),Some(&1.0)); }
                    samples.push(json!({"条件":name,"接続数":workers,"接続":worker,"回":attempt,"計画方式":mode,
                        "接続ms":connection_ms,"準備ms":preparation_ms,"往復ms":elapsed,"計画ms":plan[0]["Planning Time"],"実行ms":plan[0]["Execution Time"],
                        "走査":counts,"ブロック":root["Shared Hit Blocks"].as_u64().unwrap_or(0)+root["Shared Read Blocks"].as_u64().unwrap_or(0),
                        "計画":if attempt==0 {plan} else {Value::Null}}));
                }
            }
            client.batch_execute("DEALLOCATE ALL").await?;
        }
        Ok::<_,anyhow::Error>(samples)
    });
    Ok(try_join_all(tasks).await?.into_iter().flatten().collect())
}

async fn api_measure(config: Config, workers: usize) -> anyhow::Result<Value> {
    let at = Instant::now();
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new_for_worker(db, config));
    let preparation = milliseconds(at);
    let tasks=(0..workers).map(|worker| {
        let app=app.clone();
        async move {
            let user=format!("00000000-0000-4000-8000-{worker:012}");
            let mut samples=Vec::new();
            for attempt in 0..30 {
                let token=digest(&format!("api:{workers}:{worker}:{attempt}"));
                for kind in ["alert","voip"] {
                    let secret=digest(&format!("secret:{token}:{kind}"));
                    let body=registration(kind,"sandbox",&token,&secret);
                    let mut id=String::new();
                    for name in ["登録","再登録"] {
                        let at=Instant::now();
                        let (status,response)=post(&app,REGISTER,Some(&user),body.clone()).await?;
                        samples.push(json!({"条件":name,"用途":kind,"接続":worker,"回":attempt,"往復ms":milliseconds(at)}));
                        assert_eq!(status,axum::http::StatusCode::OK,"{name}: {response}");
                        id=response["id"].as_str().unwrap().into();
                    }
                    let at=Instant::now();
                    let (status,_)=post(&app,REVOKE,None,json!({"id":id,"revocation_secret":secret})).await?;
                    samples.push(json!({"条件":"解除","用途":kind,"接続":worker,"回":attempt,"往復ms":milliseconds(at)}));
                    assert_eq!(status,axum::http::StatusCode::OK);
                }
            }
            Ok::<_,anyhow::Error>(samples)
        }
    });
    Ok(
        json!({"接続数":workers,"準備ms":preparation,"測定":try_join_all(tasks).await?.into_iter().flatten().collect::<Vec<_>>(),
        "経路":"Router・開発用認証・実DBプール。外部ネットワーク、JWT、APNs送信は対象外。プール待ちを含む"}),
    )
}

async fn migrate_with_writes(url: &str) -> anyhow::Result<Value> {
    let ddl = connect(url).await?;
    let writer = connect(url).await?;
    let (create, _) = migration_statements();
    let at = Instant::now();
    ddl.batch_execute(create).await?;
    let build_without_barrier = milliseconds(at);
    ddl.batch_execute("DROP INDEX CONCURRENTLY apns_subscriptions_token_environment_kind_key")
        .await?;
    let reader = connect(url).await?;
    let reader_pid: i32 = reader
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let ddl_pid: i32 = ddl.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    reader
        .batch_execute(
            "BEGIN ISOLATION LEVEL REPEATABLE READ; SELECT id FROM apns_subscriptions LIMIT 1",
        )
        .await?;
    let build = async {
        let at = Instant::now();
        ddl.batch_execute(create).await?;
        Ok::<_, anyhow::Error>(milliseconds(at))
    };
    let writes = async {
        // 古いスナップショットで有効化直前に停止させ、速い環境でも更新と構築の重なりを保証する。
        let at = Instant::now();
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while !waiting_for_snapshot(&writer, ddl_pid, reader_pid).await? {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        let before_wait = milliseconds(at);
        let ready: bool = writer.query_one("SELECT indisready AND NOT indisvalid FROM pg_index WHERE indexrelid='apns_subscriptions_token_environment_kind_key'::regclass",&[]).await?.get(0);
        assert!(ready);
        let held = Instant::now();
        let mut times = Vec::new();
        for _ in 0..30 {
            assert!(waiting_for_snapshot(&writer, ddl_pid, reader_pid).await?);
            let at = Instant::now();
            writer
                .execute(
                    "UPDATE apns_subscriptions SET created_at=clock_timestamp() WHERE id=$1",
                    &[&digest(&format!("sandbox:{}", digest("1")))],
                )
                .await?;
            times.push(milliseconds(at));
        }
        assert!(waiting_for_snapshot(&writer, ddl_pid, reader_pid).await?);
        reader.batch_execute("COMMIT").await?;
        Ok::<_, anyhow::Error>(
            json!({"有効化待機前ms":before_wait,"有効化待機中の保持ms":milliseconds(held),"有効化待機中の更新ms":times,"段階":"waiting for old snapshots"}),
        )
    };
    let (controlled_ms, writes) = tokio::try_join!(build, writes)?;
    let valid: bool = ddl.query_one("SELECT indisvalid FROM pg_index WHERE indexrelid='apns_subscriptions_token_environment_kind_key'::regclass",&[]).await?.get(0);
    assert!(valid);
    let timeout = measure_constraint_lock(url, &ddl, &writer, false).await?;
    let drop = measure_constraint_lock(url, &ddl, &writer, true).await?;
    Ok(
        json!({"非注入の索引構築ms":build_without_barrier,"保持ありの索引構築ms":controlled_ms,"有効化前の並行更新":writes,"制約削除の待機上限":timeout,"制約削除ロック":drop}),
    )
}

async fn waiting_for_snapshot(
    client: &Client,
    ddl_pid: i32,
    reader_pid: i32,
) -> anyhow::Result<bool> {
    Ok(client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_progress_create_index WHERE pid=$1 AND phase='waiting for old snapshots' AND current_locker_pid=$2::integer)",&[&ddl_pid,&reader_pid]).await?.get(0))
}

async fn wait_for_lock(client: &Client, pid: i32, mode: &str) -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: bool = client.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND relation='apns_subscriptions'::regclass AND mode=$2 AND NOT granted)", &[&pid,&mode]).await?.get(0);
            if waiting { return Ok::<_,anyhow::Error>(()); }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }).await??;
    Ok(())
}

async fn measure_constraint_lock(
    url: &str,
    ddl: &Client,
    writer: &Client,
    release: bool,
) -> anyhow::Result<Value> {
    let observer = connect(url).await?;
    let queued = connect(url).await?;
    let ddl_pid: i32 = ddl.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    let write_pid: i32 = queued
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let original_timeout: String = ddl.query_one("SHOW lock_timeout", &[]).await?.get(0);
    let sql = "UPDATE apns_subscriptions SET created_at=clock_timestamp() WHERE id=$1";
    writer.batch_execute("BEGIN").await?;
    writer
        .execute(sql, &[&digest(&format!("sandbox:{}", digest("1")))])
        .await?;
    // 更新のロックを保持し、DDLと後着の更新が待機列に入ったことをpg_locksで確認する。
    let at = Instant::now();
    let mut drop = std::pin::pin!(async {
        let result = ddl.batch_execute(migration_statements().1).await;
        (result, milliseconds(at))
    });
    tokio::select! {
        result = &mut drop => { result.0?; anyhow::bail!("更新中にDDLが排他ロック待ちなしで完了しました"); }
        result = wait_for_lock(&observer,ddl_pid,"AccessExclusiveLock") => { result?; }
    }
    let mut update = std::pin::pin!(async {
        let result = queued
            .execute(sql, &[&digest(&format!("sandbox:{}", digest("2")))])
            .await;
        (result, milliseconds(at))
    });
    tokio::select! {
        result = &mut drop => { result.0?; anyhow::bail!("更新の解放前にDDLが完了しました"); }
        result = &mut update => { result.0?; anyhow::bail!("DDL待機中に後着の更新が完了しました"); }
        result = wait_for_lock(&observer,write_pid,"RowExclusiveLock") => { result?; }
    }
    let injected_wait = milliseconds(at);
    if release {
        writer.batch_execute("COMMIT").await?;
    }
    let ((result, ddl_ms), (updated, update_ms)) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(drop, update)
        })
        .await?;
    assert_eq!(updated?, 1);
    let restored: String = ddl.query_one("SHOW lock_timeout", &[]).await?.get(0);
    assert_eq!(original_timeout, restored);
    if !release {
        assert_eq!(
            result.unwrap_err().code(),
            Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE)
        );
        let protected: bool = ddl.query_one("SELECT EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='apns_subscriptions'::regclass AND conname='apns_subscriptions_device_token_environment_key')",&[]).await?.get(0);
        assert!(protected);
        writer.batch_execute("COMMIT").await?;
        return Ok(
            json!({"上限で待機解除ms":ddl_ms,"後着更新完了ms":update_ms,"設定復元":true,"旧制約を保持":protected}),
        );
    }
    result?;
    Ok(
        json!({"DDL待機を確認":true,"後着更新の待機を確認":true,"意図的なロック保持ms":injected_wait,
        "DDL完了ms":ddl_ms,"後着更新完了ms":update_ms,"保持終了後DDLms":ddl_ms-injected_wait,"保持終了後更新ms":update_ms-injected_wait}),
    )
}

async fn check_failed_build(url: &str) -> anyhow::Result<Value> {
    let client = connect(url).await?;
    // 故障注入だけに時間制限を使い、性能の判定には使わない。
    client.batch_execute("SET statement_timeout='40ms'").await?;
    let error = client
        .batch_execute(migration_statements().0)
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
    );
    client.batch_execute("SET statement_timeout=0").await?;
    let invalid: bool = client.query_one("SELECT NOT indisvalid FROM pg_index WHERE indexrelid='apns_subscriptions_token_environment_kind_key'::regclass", &[]).await?.get(0);
    let protected: bool = client.query_one("SELECT EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='apns_subscriptions'::regclass AND conname='apns_subscriptions_device_token_environment_key' AND contype='u')", &[]).await?.get(0);
    assert!(invalid && protected);
    // 無効な索引が残ったままでは、同じマイグレーションを再実行できない。
    let error = client
        .batch_execute(migration_statements().0)
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::DUPLICATE_TABLE)
    );
    client
        .batch_execute("DROP INDEX CONCURRENTLY apns_subscriptions_token_environment_kind_key")
        .await?;
    Ok(json!({"旧制約を保持":protected,"無効索引を確認":invalid,"無効索引を除去して再開":true}))
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    let output = std::env::var("ZOO_APNS_KIND_BUDGET_OUTPUT")?;
    let machine = std::env::var("ZOO_APNS_KIND_BUDGET_MACHINE")?;
    let client = connect(&config.database_url).await?;
    let mut results = Vec::new();
    for total in [300_000, 1_000_000] {
        // 正式スキーマの索引・外部キー・解除トリガーを使い、対象ユーザーは上限20件に偏らせる。
        client.batch_execute("TRUNCATE apns_subscriptions,apns_revocations;
            ALTER TABLE apns_subscriptions ADD CONSTRAINT apns_subscriptions_device_token_environment_key UNIQUE(device_token,environment);
            DROP INDEX apns_subscriptions_token_environment_kind_key;").await?;
        let at = Instant::now();
        client.batch_execute(&format!("INSERT INTO users(id,user_name,display_name) SELECT 'budget-u'||n,'budget-u'||n,'性能検証' FROM generate_series(1,{total}/20) n ON CONFLICT DO NOTHING;
            INSERT INTO users(id,user_name,display_name) SELECT '00000000-0000-4000-8000-'||lpad(n::text,12,'0'),'budget-api-'||n,'性能検証' FROM generate_series(0,7) n ON CONFLICT DO NOTHING;
            INSERT INTO apns_subscriptions(id,user_id,device_token,environment,revocation_hash,created_at)
              SELECT encode(sha256(('sandbox:'||encode(sha256(n::text::bytea),'hex'))::bytea),'hex'),
                'budget-u'||((n-1)/20+1),encode(sha256(n::text::bytea),'hex'),'sandbox','検証用',now()
              FROM generate_series(1,{total}) n;
            ANALYZE apns_subscriptions; ANALYZE users;")).await?;
        let fixture_ms = milliseconds(at);
        let old_bytes: i64 = client
            .query_one(
                "SELECT pg_relation_size('apns_subscriptions_device_token_environment_key')",
                &[],
            )
            .await?
            .get(0);
        let old_sql = sql_measure(&config.database_url, 1).await?;
        let recovery = check_failed_build(&config.database_url).await?;
        let migration = migrate_with_writes(&config.database_url).await?;
        client.batch_execute("ANALYZE apns_subscriptions").await?;
        let new_bytes: i64 = client
            .query_one(
                "SELECT pg_relation_size('apns_subscriptions_token_environment_kind_key')",
                &[],
            )
            .await?
            .get(0);
        let mut sql = sql_measure(&config.database_url, 1).await?;
        sql.extend(sql_measure(&config.database_url, 8).await?);
        let mut api = Vec::new();
        for workers in [1, 8] {
            api.push(api_measure(config.clone(), workers).await?);
        }
        results.push(json!({"件数":total,"準備ms":fixture_ms,"旧索引bytes":old_bytes,"新索引bytes":new_bytes,"移行":migration,"失敗からの再開":recovery,"移行前SQL":old_sql,"SQL":sql,"API":api}));
    }
    let settings:String=client.query_one("SELECT json_object_agg(name,json_build_object('値',setting,'単位',unit))::text FROM pg_settings WHERE name IN ('server_version','shared_buffers','work_mem','effective_cache_size','max_connections','random_page_cost','effective_io_concurrency','jit','default_statistics_target','max_parallel_workers_per_gather','plan_cache_mode')",&[]).await?.get(0);
    std::fs::write(
        output,
        serde_json::to_vec(
            &json!({"機器":machine,"設定":serde_json::from_str::<Value>(&settings)?,"キャッシュ":"OS・DBキャッシュの強制破棄なし。初回も含む。各fixture後にANALYZE","結果":results}),
        )?,
    )?;
    // 更新のHOT化や索引ページ分割の発生回は一致しないため、条件別の最悪値を比較する。
    let maxima = |rows: &Value| {
        let mut values = BTreeMap::<String, u64>::new();
        for row in rows.as_array().unwrap() {
            let key = format!("{}:{}:{}", row["条件"], row["接続数"], row["計画方式"]);
            let value = values.entry(key).or_default();
            *value = (*value).max(row["ブロック"].as_u64().unwrap());
        }
        values
    };
    let small = maxima(&results[0]["SQL"]);
    for (condition, large) in maxima(&results[1]["SQL"]) {
        anyhow::ensure!(
            large <= 2 * small[&condition] + 16,
            "履歴に比例する論理ブロックの増加: 条件={condition} 小={} 大={large}",
            small[&condition]
        );
    }
    Ok(())
}
