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
            if name == "directory"
                && !(sql.contains("FROM sip_lines WHERE created_by=$1")
                    || sql == "SELECT to_json(id)::text FROM sip_phones WHERE line_id=$1"
                    || sql.contains("ORDER BY l.name,l.id"))
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
            let max = (1..=5)
                .filter(|i| sql.contains(&format!("${i}")))
                .max()
                .unwrap_or(0);
            let prepared = format!(
                "PREPARE budget({}) AS {sql}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget({});",
                vec!["text"; max].join(","),
                args[..max]
                    .iter()
                    .map(|v| quote(v))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            queries.push((format!("{name}-{}", queries.len()), prepared));
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
        queries.len() >= 24,
        "new SQL must stay covered: {}",
        queries.len()
    );
    let queries = Arc::new(queries);
    let metrics = Arc::new(Mutex::new(Vec::<Value>::new()));
    // First execution remains in the sample. Single then eight simultaneous connections, 30 repeats each.
    for concurrency in [1, 8] {
        let mut tasks = Vec::new();
        for _ in 0..concurrency {
            let (url, queries, metrics) = (url.clone(), queries.clone(), metrics.clone());
            tasks.push(tokio::spawn(async move {
                let db=client(&url).await.unwrap();
                for sample in 0..30 {for (name,sql) in queries.iter() {
                    let start=std::time::Instant::now();
                    // Each plan includes the complete write, FK checks and locks, but changes roll back.
                    let rows=db.simple_query(&format!("BEGIN; {sql} ROLLBACK; DEALLOCATE ALL;")).await.unwrap();
                    let raw=rows.iter().find_map(|row|match row {SimpleQueryMessage::Row(row)=>row.get(0).filter(|s|s.starts_with('[')),_=>None}).unwrap();
                    let value:Value=serde_json::from_str(raw).unwrap();let plan=&value[0];let mut relations=std::collections::BTreeMap::new();scan(&plan["Plan"],&mut relations);
                    for (relation,work) in &relations {let bound=if relation=="sip_phones" {100.} else if relation=="phone_sessions" {12.} else {8.};assert!(*work<=bound,"{name} {relation}: {work}>{bound}: {plan}");}
                    metrics.lock().unwrap().push(json!({"name":name,"concurrency":concurrency,"plan":if sample==0 {plan.clone()} else {Value::Null},"relations":relations,"blocks":plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)+plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0),"planning_ms":plan["Planning Time"],"execution_ms":plan["Execution Time"],"roundtrip_ms":start.elapsed().as_secs_f64()*1000.}));
                }}
            }));
        }
        for task in tasks {
            task.await?;
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
            &json!({"rows":n,"queries":queries.len(),"samples":*metrics.lock().unwrap(),"api_samples":*api_metrics.lock().unwrap()}),
        )?,
    )?;
    println!(
        "phone query budget: {n} rows, {} queries, output {output}",
        queries.len()
    );
    Ok(())
}
