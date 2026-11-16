//! Disposable migrated DB only: PHONE_SCALE_ROWS=300000/1000000 and DATABASE_URL.
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_postgres::{NoTls, SimpleQueryMessage};
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
fn scan(node: &Value, relations: &mut std::collections::BTreeMap<String, f64>) {
    let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
    let work = (node["Actual Rows"].as_f64().unwrap_or(0.)
        + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
        + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
        * loops;
    if let Some(name) = node["Relation Name"].as_str() {
        *relations.entry(name.into()).or_default() += work;
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
// Capture actual RI/CASCADE plans; the parent EXPLAIN omits their buffers.
async fn plan_client(
    url: &str,
) -> anyhow::Result<(tokio_postgres::Client, Arc<Mutex<Vec<Value>>>)> {
    let (client, mut connection) = tokio_postgres::connect(url, NoTls).await?;
    let plans = Arc::new(Mutex::new(Vec::new()));
    let output = plans.clone();
    tokio::spawn(async move {
        while let Some(message) = std::future::poll_fn(|cx| connection.poll_message(cx)).await {
            if let tokio_postgres::AsyncMessage::Notice(notice) = message.unwrap() {
                if let Some(at) = notice.message().find('{') {
                    if let Ok(plan) = serde_json::from_str::<Value>(&notice.message()[at..]) {
                        if plan["Query Text"]
                            .as_str()
                            .is_some_and(|sql| sql.contains("FROM ONLY"))
                        {
                            output.lock().unwrap().push(plan);
                        }
                    }
                }
            }
        }
    });
    client.batch_execute("LOAD 'auto_explain'; SET auto_explain.log_min_duration=0; SET auto_explain.log_analyze=on; SET auto_explain.log_timing=off; SET auto_explain.log_buffers=on; SET auto_explain.log_nested_statements=on; SET auto_explain.log_format=json; SET auto_explain.log_level=notice;").await?;
    Ok((client, plans))
}
#[tokio::test]
#[ignore = "requires an empty migrated disposable database and PHONE_SCALE_ROWS"]
async fn phone_queries_have_bounded_work() -> anyhow::Result<()> {
    let n: usize = std::env::var("PHONE_SCALE_ROWS")?.parse()?;
    assert!(n >= 300000);
    let url = std::env::var("DATABASE_URL")?;
    let db = client(&url).await?;
    let actor = "550e8400-e29b-41d4-a716-446655440001";
    let noise = "550e8400-e29b-41d4-a716-446655440002";
    let line = "550e8400-e29b-41d4-a716-446655440003";
    let session = "550e8400-e29b-41d4-a716-446655440004";
    let phone = "550e8400-e29b-41d4-a716-446655440005";
    if std::env::var("PHONE_SCALE_REUSE").as_deref() != Ok("1") {
        db.batch_execute(&format!("INSERT INTO users(id,user_name,display_name) VALUES('{actor}','phone-budget-actor','Budget'),('{noise}','phone-budget-noise','Budget'); INSERT INTO sip_lines(id,name,owner_user_id,websocket_path,sip_domain,username,password_ciphertext,created_by) VALUES('{line}','Target','{actor}','/sip/ws','pbx.example','alice','dummy','{actor}'); INSERT INTO sip_lines(id,name,owner_user_id,websocket_path,sip_domain,username,password_ciphertext,created_by) SELECT 'noise-'||i,'Noise','{noise}','/sip/ws','pbx.example','noise','dummy','{noise}' FROM generate_series(1,{n}) i; INSERT INTO sip_phones(id,line_id,name,extension) SELECT 'noise-'||i,'noise-1','Noise',i::text FROM generate_series(1,{n}) i; INSERT INTO sip_phones(id,line_id,name,extension) SELECT CASE WHEN i=1 THEN '{phone}' ELSE 'target-'||i END,'{line}','Target',i::text FROM generate_series(1,100) i; INSERT INTO phone_sessions(id,user_id,line_id,line_revision,peer_id,expires_at) SELECT 'noise-'||i,'{noise}','noise-1',now(),'noise',now()-interval '1 hour' FROM generate_series(1,{n}) i; INSERT INTO phone_sessions(id,user_id,line_id,line_revision,peer_id,gateway_session_id,gateway_handle_id,expires_at) SELECT CASE WHEN i=1 THEN '{session}' ELSE 'target-'||i END,'{actor}','{line}',updated_at,'peer',1,2,now()+CASE WHEN i<=2 THEN interval '1 day' ELSE interval '-1 day' END FROM sip_lines CROSS JOIN generate_series(1,4) i WHERE id='{line}'; ANALYZE;" )).await?;
    }
    // Membership history grows too; cover both owner and shared-member authorization.
    if db
        .query_one(
            "SELECT count(*) FROM sip_line_members WHERE user_id=$1",
            &[&noise],
        )
        .await?
        .get::<_, i64>(0)
        == 0
    {
        db.batch_execute(&format!("INSERT INTO sip_line_members(line_id,user_id) SELECT 'noise-'||i,'{noise}' FROM generate_series(1,{n}) i; INSERT INTO sip_lines(id,name,owner_user_id,websocket_path,sip_domain,username,password_ciphertext,created_by) VALUES('550e8400-e29b-41d4-a716-446655440006','Shared',NULL,'/sip/ws','pbx.example','shared','dummy','{actor}'); INSERT INTO sip_line_members(line_id,user_id) VALUES('550e8400-e29b-41d4-a716-446655440006','{actor}'); UPDATE phone_sessions SET line_id='550e8400-e29b-41d4-a716-446655440006',line_revision=(SELECT updated_at FROM sip_lines WHERE id='550e8400-e29b-41d4-a716-446655440006') WHERE id='target-2'; ANALYZE;" )).await?;
    }
    db.batch_execute(&format!("INSERT INTO sip_lines(id,name,owner_user_id,websocket_path,sip_domain,username,password_ciphertext,created_by) SELECT 'admin-'||i,'Admin '||i,'{noise}','/sip/ws','pbx.example','noise','dummy','{noise}' FROM generate_series(1,200) i ON CONFLICT DO NOTHING; UPDATE sip_lines SET owner_user_id='{noise}',created_by='{noise}' WHERE id IN (SELECT 'admin-'||i FROM generate_series(1,200) i); ANALYZE sip_lines;")).await?;
    if db
        .query_opt("SELECT id FROM users WHERE id='phone-noise-user-1'", &[])
        .await?
        .is_none()
    {
        db.batch_execute(&format!("INSERT INTO users(id,user_name,display_name) SELECT 'phone-noise-user-'||i,'phone-budget-noise-user-'||i,'Budget' FROM generate_series(1,{n}) i;")).await?;
    }
    db.batch_execute("ANALYZE users;").await?;
    // Exercise full per-line child limits, including FK CASCADE on line removal.
    db.batch_execute(&format!("INSERT INTO users(id,user_name,display_name) SELECT 'phone-member-'||i,'phone-budget-member-'||i,'Budget' FROM generate_series(1,100) i ON CONFLICT DO NOTHING; INSERT INTO sip_line_members(line_id,user_id) SELECT '{line}','phone-member-'||i FROM generate_series(1,100) i ON CONFLICT DO NOTHING; ANALYZE sip_line_members;")).await?;
    let revision = db
        .query_one(
            "SELECT updated_at::text FROM sip_lines WHERE id=$1",
            &[&line],
        )
        .await?
        .get::<_, String>(0);
    let mut queries = Vec::<(String, String)>::new();
    for (name, source) in [
        ("sessions", include_str!("../src/routes/phone_sessions.rs")),
        ("pbx", include_str!("../src/routes/pbx.rs")),
        ("directory", include_str!("../src/routes/sip.rs")),
    ] {
        for sql in source.split('"').filter(|s| {
            matches!(
                s.split_whitespace().next(),
                Some("SELECT" | "UPDATE" | "DELETE" | "INSERT")
            )
        }) {
            if !sql.contains("FROM ") && !sql.starts_with("INSERT ") && !sql.starts_with("UPDATE ")
            {
                continue;
            }
            let mut args = vec![
                session.to_owned(),
                actor.to_owned(),
                "owner".into(),
                "2".into(),
                "peer".into(),
            ];
            if sql.contains("FROM users WHERE id=$1") || sql.contains("WHERE user_id=$1") {
                args[0] = actor.into();
            }
            if sql.contains("FROM sip_lines l WHERE") {
                args[0] = line.into();
                args[2] = revision.clone();
            }
            if sql.starts_with("INSERT INTO phone_sessions") {
                args[0] = "new-session".into();
                args[2] = line.into();
                args[3] = revision.clone();
            }
            if sql.contains("gateway_session_id=$3") {
                args[2] = "1".into();
            }
            if name == "pbx" {
                args[0] = if sql.contains("FROM sip_phones p") {
                    phone
                } else {
                    line
                }
                .into();
                args[1] = "1".into();
            }
            if name == "directory" {
                args[0] = if sql.contains("created_by=$1") {
                    actor
                } else {
                    line
                }
                .into();
            }
            if name == "directory" && sql.contains("ORDER BY l.name,l.id") {
                args[0] = actor.into();
                args[1] = "false".into();
            }
            // Parse complete placeholder numbers: $10 must not be mistaken for $1.
            let max = sql
                .split('$')
                .skip(1)
                .filter_map(|part| {
                    part.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<usize>()
                        .ok()
                })
                .max()
                .unwrap_or(0);
            let mut types = vec!["text"; max];
            if name == "directory" {
                let values: Vec<String> = if sql.starts_with("INSERT INTO sip_lines") {
                    types[2] = "boolean";
                    vec![
                        "new-line",
                        "New",
                        "false",
                        actor,
                        "/sip/ws",
                        "pbx.example",
                        "alice",
                        "dummy",
                        "123",
                    ]
                } else if sql.starts_with("UPDATE sip_lines") {
                    vec![
                        line,
                        "Updated",
                        "/sip/ws",
                        "pbx.example",
                        "alice",
                        "dummy",
                        "123",
                    ]
                } else if sql.starts_with("INSERT INTO sip_phones") {
                    vec!["new-phone", line, "New", "new-extension"]
                } else if sql.starts_with("INSERT INTO sip_line_members") {
                    vec![line, actor]
                } else if sql.contains("ORDER BY l.name,l.id") {
                    vec![actor, "false"]
                } else if sql.contains("FROM users WHERE id=$1") || sql.contains("created_by=$1") {
                    vec![actor]
                } else if sql.contains("$2") {
                    assert!(
                        sql.contains("FROM sip_lines l WHERE id=$1"),
                        "unclassified SQL: {sql}"
                    );
                    vec![line, actor]
                } else {
                    assert!(
                        sql.contains("FROM sip_lines WHERE id=$1")
                            || sql.contains("FROM sip_phones WHERE line_id=$1")
                            || sql.starts_with("DELETE FROM sip_"),
                        "unclassified SQL: {sql}"
                    );
                    vec![line]
                }
                .into_iter()
                .map(str::to_owned)
                .collect();
                assert_eq!(values.len(), max, "argument contract changed: {sql}");
                args = values;
            }
            let prepared = format!(
                "PREPARE budget({}) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({});",
                types.join(","),
                args[..max]
                    .iter()
                    .map(|v| quote(v))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            queries.push((format!("{name}-{}", queries.len()), prepared));
            if name == "directory" && sql == "DELETE FROM sip_lines WHERE id=$1" {
                queries.push(("cascade-403".into(), format!("SELECT id FROM sip_lines WHERE id={} FOR UPDATE; INSERT INTO phone_sessions(id,user_id,line_id,line_revision,peer_id,expires_at) SELECT 'cascade-'||i,'phone-member-'||((i-1)/4+1),l.id,l.updated_at,'peer',now()-interval '1 hour' FROM sip_lines l CROSS JOIN generate_series(1,400) i WHERE l.id={}; PREPARE budget(text) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({});",quote(line),quote(line),quote(line))));
            }
            if name == "directory" && sql.contains("FROM sip_lines WHERE created_by=$1") {
                queries.push(("creator-limit".into(), format!("UPDATE sip_lines SET created_by={} WHERE id IN (SELECT 'admin-'||i FROM generate_series(1,48) i); PREPARE budget(text) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({});", quote(actor),quote(actor))));
            }
            if name == "directory" && sql.contains("ORDER BY l.name,l.id") {
                // Keep total history N fixed while varying visible shared lines L.
                // Fixture changes run before the measured group and reset after it.
                for visible in [20, 200] {
                    queries.push((format!("admin-{visible}"), format!(
                        "UPDATE sip_lines SET owner_user_id=NULL WHERE id IN (SELECT 'admin-'||i FROM generate_series(1,{visible}) i); PREPARE budget(text,text) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({},'true');", quote(actor))));
                }
            }
            if sql.contains("JOIN sip_lines l ON l.id=s.line_id") {
                for (suffix, first, second) in [
                    ("private", session, noise),
                    ("missing", "missing-session", actor),
                    ("shared", "target-2", actor),
                ] {
                    queries.push((format!("owned-{suffix}"),format!("PREPARE budget(text,text) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({},{});",quote(first),quote(second))));
                }
            }
            if sql.contains("FROM sip_phones WHERE line_id=$1 AND extension=$2") {
                queries.push(("empty-extension".into(),format!("PREPARE budget(text,text) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({},'missing');",quote(line))));
            }
        }
    }
    assert!(
        queries.len() >= 41,
        "new SQL must stay covered: {}",
        queries.len()
    );
    let queries = Arc::new(queries);
    let metrics = Arc::new(Mutex::new(Vec::<Value>::new()));
    // Set up cardinality variants once, before timing. Read-only admin queries
    // then run concurrently without fixture UPDATE locks or artificial deadlocks.
    for (name, definition) in queries.iter() {
        let variant = name.starts_with("admin-") || name == "creator-limit";
        let sql = if variant {
            let at = definition.find("PREPARE ").unwrap();
            db.batch_execute(&definition[..at]).await?;
            db.batch_execute("ANALYZE sip_lines;").await?;
            definition[at..].to_owned()
        } else {
            definition.clone()
        };
        // First execution remains in the sample. Single then eight connections, 30 repeats.
        for concurrency in [1, 8] {
            let mut tasks = Vec::new();
            for _ in 0..concurrency {
                let (url, name, sql, metrics) =
                    (url.clone(), name.clone(), sql.clone(), metrics.clone());
                tasks.push(tokio::spawn(async move {
                    let (db,nested_plans)=plan_client(&url).await.unwrap();
                    for sample in 0..30 {
                    let at=sql.find("EXPLAIN ").unwrap();
                    let preparation=std::time::Instant::now();
                    db.batch_execute(&format!("BEGIN; {}",&sql[..at])).await.unwrap();
                    let preparation_ms=preparation.elapsed().as_secs_f64()*1000.;
                    nested_plans.lock().unwrap().clear();
                    let start=std::time::Instant::now();
                    let rows=db.simple_query(&format!("{} ROLLBACK; DEALLOCATE ALL;",&sql[at..])).await.unwrap();
                    let nested=std::mem::take(&mut *nested_plans.lock().unwrap());
                    let raw=rows.iter().find_map(|row|match row {SimpleQueryMessage::Row(row)=>row.get(0).filter(|s|s.starts_with('[')),_=>None}).unwrap();
                    let value:Value=serde_json::from_str(raw).unwrap();let plan=&value[0];
                    if let Some(visible) = name.strip_prefix("admin-").and_then(|s|s.parse::<u64>().ok()) { assert_eq!(plan["Plan"]["Actual Rows"].as_u64(),Some(visible+2),"shared lines must never disappear"); }
                    let mut relations=std::collections::BTreeMap::new();scan(&plan["Plan"],&mut relations);
                    for child in &nested { scan(&child["Plan"],&mut relations); }
                    if name=="cascade-403" { assert!(nested.iter().any(|p|p["Query Text"].as_str().unwrap_or("").contains("phone_sessions")),"CASCADE plan missing"); }
                    for (relation,work) in &relations {let visible = name.strip_prefix("admin-").and_then(|s|s.parse::<f64>().ok()).unwrap_or(0.);
                    let bound=if visible > 0. { if relation=="sip_lines" {3.*(visible+2.)} else if relation=="sip_line_members" {visible+4.} else if relation=="sip_phones" {100.+visible+2.} else {8.} } else if relation=="sip_phones" {100.} else if name=="cascade-403" && relation=="phone_sessions" {403.} else if relation=="phone_sessions" {12.} else if name=="creator-limit" && relation=="sip_lines" {50.} else if (name.starts_with("directory-") || name=="cascade-403") && relation=="sip_line_members" {100.} else {8.};assert!(*work<=bound,"{name} {relation}: {work}>{bound}: {plan}");}
                    metrics.lock().unwrap().push(json!({"name":name,"concurrency":concurrency,"plan":if sample==0 {plan.clone()} else {Value::Null},"relations":relations,"nested_plans":if sample==0 {json!(nested)} else {Value::Null},"preparation_ms":preparation_ms,"blocks":plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)+plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0)+nested.iter().map(|p|p["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)+p["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0)).sum::<u64>(),"planning_ms":plan["Planning Time"],"execution_ms":plan["Execution Time"],"roundtrip_ms":start.elapsed().as_secs_f64()*1000.}));
                    }
                }));
            }
            for task in tasks {
                task.await?;
            }
        }
        if variant {
            db.batch_execute(&format!("UPDATE sip_lines SET owner_user_id='{noise}',created_by='{noise}' WHERE id IN (SELECT 'admin-'||i FROM generate_series(1,200) i); ANALYZE sip_lines;")).await?;
        }
    }
    // Includes authentication, pool wait, JSON/form serialization and the HTTP gateway hop.
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use tower::ServiceExt;
    let gateway = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"janus":"ack"})))
        .mount(&gateway)
        .await;
    let mut config = backend::config::Config::from_env();
    config.database_url = url.clone();
    config.dev_auth_enabled = true;
    config.sip_encryption_key = Some(STANDARD.encode([3; 32]));
    config.phone_gateway = backend::phone_gateway::GatewayConfig {
        url: Some(format!("{}/janus", gateway.uri())),
        api_secret: Some("dummy-api".into()),
        pbx_uri: Some("sip:127.0.0.1:5060;transport=tcp".into()),
        pbx_config_secret: Some("dummy-config".into()),
        media_mode: Some("lan".into()),
    };
    let state = backend::state::AppState::new(backend::db::init_db(&url).await?, config);
    let app = backend::routes::create_router(state);
    let api_metrics = Arc::new(Mutex::new(Vec::<Value>::new()));
    for concurrency in [1, 8] {
        let mut tasks = Vec::new();
        for _ in 0..concurrency {
            let (app, metrics) = (app.clone(), api_metrics.clone());
            tasks.push(tokio::spawn(async move {
                for _ in 0..30 {for name in ["directory","command","pbx-endpoint"] {
                    let mut request=Request::builder().header("X-Dev-User",actor);
                    let body=match name {
                        "directory"=>{ request=request.uri("/api/sip/lines"); Body::empty() },
                        "command"=>{ request=request.method("POST").uri(format!("/api/phone/sessions/{session}/command")).header("Content-Type","application/json"); Body::from(r#"{"action":"dtmf","tone":"1"}"#) },
                        _=>{ request=request.method("POST").uri("/api/internal/pbx/endpoints/single").header("Authorization","Bearer dummy-config").header("Content-Type","application/x-www-form-urlencoded"); Body::from(format!("id=phone-{phone}")) },
                    };
                    let start=std::time::Instant::now();
                    let response=app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
                    assert_eq!(response.status(),200,"{name}");
                    let bytes=to_bytes(response.into_body(),262144).await.unwrap();assert!(!bytes.is_empty());
                    metrics.lock().unwrap().push(json!({"name":name,"concurrency":concurrency,"roundtrip_ms":start.elapsed().as_secs_f64()*1000.}));
                }}
            }));
        }
        for task in tasks {
            task.await?;
        }
    }
    let output =
        std::env::var("PHONE_SCALE_OUTPUT").unwrap_or(format!("/tmp/phone-budget-{n}.json"));
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(
            &json!({"rows":n,"queries":queries.len(),"query_definitions":*queries,"samples":*metrics.lock().unwrap(),"api_samples":*api_metrics.lock().unwrap()}),
        )?,
    )?;
    println!(
        "phone query budget: {n} rows, {} queries, output {output}",
        queries.len()
    );
    Ok(())
}
