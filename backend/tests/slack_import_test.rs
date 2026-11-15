use std::io::{Cursor, Write};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures_util::FutureExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

use backend::{
    apps::query, config::Config, db::init_db, models::UserRole, routes::create_router,
    state::AppState, storage::ensure_bucket_exists,
};

/// ワーカーの run_once は DB 上のどのジョブでも拾うため、設定 (添付取得のトークンなど) の違う
/// テスト同士が互いのジョブを処理しないよう、このファイルのテストは順に実行する。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Dev-User", user);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value)?)
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(builder.body(body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Ok((status, value))
}

async fn sql(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn exec(
    tx: &mut toasty::Transaction<'_>,
    statement: &str,
    params: &[&str],
) -> anyhow::Result<()> {
    let mut query = toasty::sql::statement(statement);
    for param in params {
        query = query.bind(*param);
    }
    query.exec(tx).await?;
    Ok(())
}

async fn cleanup_statement(
    state: &AppState,
    statement: &str,
    params: &[&str],
) -> anyhow::Result<()> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    exec(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(())
}

fn finish_test_result(
    test_result: anyhow::Result<()>,
    cleanup_errors: Vec<String>,
) -> anyhow::Result<()> {
    match (test_result, cleanup_errors.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Err(test_error), true) => Err(test_error),
        (Ok(()), false) => Err(anyhow::anyhow!(
            "test succeeded, but cleanup failed: {}",
            cleanup_errors.join("; ")
        )),
        (Err(test_error), false) => Err(anyhow::anyhow!(
            "test failed: {test_error:#}; cleanup also failed: {}",
            cleanup_errors.join("; ")
        )),
    }
}

async fn create_user(app: &Router, name: &str) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        app,
        "POST",
        "/api/users",
        &id,
        Some(json!({
            "id": id,
            "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()),
            "display_name": name,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(id)
}

fn build_zip(files: &[(String, Value)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, value) in files {
            zip.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(value.to_string().as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

/// ワーカーを回し、ジョブが目的の状態になるまで待つ
async fn drive(
    state: &AppState,
    app: &Router,
    admin: &str,
    id: &str,
    until: &str,
) -> anyhow::Result<Value> {
    for _ in 0..200 {
        let (_, job) = request(
            app,
            "GET",
            &format!("/api/admin/slack-imports/{id}"),
            admin,
            None,
        )
        .await
        .map_err(|error| anyhow::anyhow!("reading import job {id}: {error}"))?;
        if job["status"] == until {
            return Ok(job);
        }
        anyhow::ensure!(job["status"] != "failed", "import failed: {job}");
        if !backend::slack::worker::run_once(state).await? {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    anyhow::bail!("import {id} did not reach {until}");
}

async fn drive_checked(
    state: &AppState,
    app: &Router,
    admin: &str,
    id: &str,
    until: &str,
) -> anyhow::Result<Value> {
    for _ in 0..200 {
        let (_, job) = request(
            app,
            "GET",
            &format!("/api/admin/slack-imports/{id}"),
            admin,
            None,
        )
        .await?;
        if job["status"] == until {
            return Ok(job);
        }
        anyhow::ensure!(job["status"] != "failed", "import failed: {job}");
        if !backend::slack::worker::run_once(state).await? {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    anyhow::bail!("import {id} did not reach {until}")
}

/// ZIP を作ってアップロードし、解析まで進める
#[derive(Clone, Copy)]
enum UploadCleanupFailure {
    None,
    DatabaseError,
    S3Error,
    DatabaseTimeout,
    S3Timeout,
}

async fn upload(state: &AppState, app: &Router, admin: &str, zip: &[u8]) -> anyhow::Result<Value> {
    upload_with_cleanup_failure(state, app, admin, zip, UploadCleanupFailure::None).await
}

async fn upload_with_cleanup_failure(
    state: &AppState,
    app: &Router,
    admin: &str,
    zip: &[u8],
    cleanup_failure: UploadCleanupFailure,
) -> anyhow::Result<Value> {
    let mut job_id = None;
    let result: anyhow::Result<Value> = async {
        let (status, created) = request(
            app,
            "POST",
            "/api/admin/slack-imports",
            admin,
            Some(json!({"file_name": "export.zip", "file_size": zip.len()})),
        )
        .await?;
        anyhow::ensure!(
            status == StatusCode::OK,
            "upload creation failed: {created}"
        );
        let id = created["import"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("upload response has no job id: {created}"))?
            .to_string();
        job_id = Some(id.clone());
        let put =
            reqwest::Client::new()
                .put(created["upload_url"].as_str().ok_or_else(|| {
                    anyhow::anyhow!("upload response has no upload URL: {created}")
                })?)
                .header("content-type", "application/zip")
                .body(zip.to_vec())
                .send()
                .await?;
        anyhow::ensure!(
            put.status().is_success(),
            "ZIP upload failed: {}",
            put.text().await.unwrap_or_default()
        );
        if !matches!(cleanup_failure, UploadCleanupFailure::None) {
            anyhow::bail!("injected upload failure after ZIP object creation");
        }
        let (status, job) = request(
            app,
            "POST",
            &format!("/api/admin/slack-imports/{id}/uploaded"),
            admin,
            None,
        )
        .await?;
        anyhow::ensure!(status == StatusCode::OK, "upload completion failed: {job}");
        let ready = drive_checked(state, app, admin, &id, "ready").await?;
        anyhow::ensure!(
            ready["id"].as_str() == Some(id.as_str()),
            "ready upload response does not match created job {id}: {ready}"
        );
        Ok(ready)
    }
    .await;
    if result.is_err()
        && let Some(id) = job_id.as_deref()
    {
        let key = format!("slack-imports/{id}/export.zip");
        let mut cleanup_errors = Vec::new();
        macro_rules! attempt_upload_cleanup {
            ($label:expr, $future:expr) => {
                match tokio::time::timeout(std::time::Duration::from_secs(5), $future).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => cleanup_errors.push(format!("{}: {error:#}", $label)),
                    Err(_) => cleanup_errors.push(format!("{}: timed out after 5s", $label)),
                }
            };
        }
        // An upload may be the only durable pointer to this ZIP. Keep the job until
        // object removal is confirmed so failed/uncertain S3 cleanup stays recoverable.
        match cleanup_failure {
            UploadCleanupFailure::S3Error => {
                attempt_upload_cleanup!("injected S3 cleanup failure", async {
                    state
                        .s3_client
                        .delete_object()
                        .bucket("invalid bucket name")
                        .key(&key)
                        .send()
                        .await?;
                    Ok::<(), anyhow::Error>(())
                })
            }
            UploadCleanupFailure::S3Timeout => {
                attempt_upload_cleanup!("injected S3 cleanup timeout", async {
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    Ok::<(), anyhow::Error>(())
                })
            }
            _ => attempt_upload_cleanup!("delete uploaded ZIP object", async {
                state
                    .s3_client
                    .delete_object()
                    .bucket(&state.config.s3.bucket)
                    .key(&key)
                    .send()
                    .await?;
                Ok::<(), anyhow::Error>(())
            }),
        }
        let object_removed = if cleanup_errors.is_empty() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                assert_s3_object_exists(&state.s3_client, &state.config.s3.bucket, &key, false),
            )
            .await
            {
                Ok(Ok(())) => true,
                Ok(Err(error)) => {
                    cleanup_errors.push(format!(
                        "verify uploaded ZIP object removal from bucket {} at key {key}: {error:#}",
                        state.config.s3.bucket
                    ));
                    false
                }
                Err(_) => {
                    cleanup_errors.push(format!("verify uploaded ZIP object removal from bucket {} at key {key}: timed out after 5s", state.config.s3.bucket));
                    false
                }
            }
        } else {
            false
        };
        if object_removed {
            match cleanup_failure {
                UploadCleanupFailure::DatabaseError => attempt_upload_cleanup!(
                    "injected database cleanup failure",
                    cleanup_statement(state, "SELECT 1 / 0", &[])
                ),
                UploadCleanupFailure::DatabaseTimeout => {
                    attempt_upload_cleanup!("injected database cleanup timeout", async {
                        tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                        Ok::<(), anyhow::Error>(())
                    })
                }
                _ => {}
            }
            if !matches!(
                cleanup_failure,
                UploadCleanupFailure::DatabaseError | UploadCleanupFailure::DatabaseTimeout
            ) {
                attempt_upload_cleanup!("delete upload job", async {
                    cleanup_statement(state, "DELETE FROM slack_import_jobs WHERE id=$1", &[id])
                        .await
                });
            }
        }
        if !cleanup_errors.is_empty() {
            let upload_error = result
                .as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "unknown upload failure".to_string());
            return Err(anyhow::anyhow!(
                "upload failed ({upload_error}); recovery resources: job_id={id}, bucket={}, key={key}; cleanup failed: {}",
                state.config.s3.bucket,
                cleanup_errors.join("; ")
            ));
        }
    }
    result
}

/// 保存したエクスポートから取り込みを始め、取り込みのジョブを返す
async fn start(
    app: &Router,
    admin: &str,
    export: &str,
    mappings: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    request(
        app,
        "POST",
        &format!("/api/admin/slack-imports/{export}/start"),
        admin,
        Some(json!({ "mappings": mappings })),
    )
    .await
}

#[tokio::test]
async fn test_slack_export_import_flow() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/F1/notes.txt"))
        .and(header("authorization", "Bearer xoxb-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_bytes(b"hello file".to_vec()),
        )
        .mount(&files)
        .await;
    // F3 は 3 回失敗してから取得できるようになる (トークンの権限を直した想定)
    Mock::given(method("GET"))
        .and(path("/files/F3/broken.txt"))
        .respond_with(ResponseTemplate::new(403))
        .up_to_n_times(3)
        .with_priority(1)
        .mount(&files)
        .await;
    Mock::given(method("GET"))
        .and(path("/files/F3/broken.txt"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_bytes(b"fixed".to_vec()),
        )
        .with_priority(2)
        .mount(&files)
        .await;

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = Some("xoxb-test".into());
    config.slack.files_base_url = format!("{}/files/", files.uri());
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());

    let admin = create_user(&app, "slack_admin").await?;
    let member = create_user(&app, "slack_member").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    // 一般ユーザーは取り込めない
    let (status, _) = request(
        &app,
        "POST",
        "/api/admin/slack-imports",
        &member,
        Some(json!({"file_name": "export.zip", "file_size": 10})),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 既存の zoo チャンネル (取り込み先として選ぶ)
    let existing = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &app,
        "POST",
        "/api/channels",
        &admin,
        Some(json!({
            "id": existing,
            "channel_name": format!("existing_{}", uuid::Uuid::new_v4().simple()),
            "is_private": false,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    // 既存チャンネルには、取り込む過去ログより新しい通常の投稿があり、管理者はまだ読んでいない
    let newer = uuid::Uuid::new_v4().to_string();
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement(
            "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'newer','2026-01-01 00:00:00')",
        )
        .bind(newer.as_str())
        .bind(admin.as_str())
        .bind(existing.as_str())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let random = format!("random-{suffix}");
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([
                {"id":"U1","team_id":team,"name":"alice","real_name":"Alice A","profile":{"display_name":"alice","image_72":"https://avatars.example/a.png"}},
                {"id":"U2","team_id":team,"name":"admin","real_name":"Bob B"}
            ]),
        ),
        (
            "channels.json".into(),
            json!([
                {"id":"C1","name":"general","created":1704067200,"members":["U1","U2"],"purpose":{"value":"Company wide"}},
                {"id":"C2","name":random,"members":["U1"]}
            ]),
        ),
        (
            "general/2024-01-01.json".into(),
            json!([
                {"ts":"1704070000.000100","user":"U1","text":"hello <@U2> see <#C2|random>","thread_ts":"1704070000.000100",
                 "reactions":[{"name":"+1","users":["U2","U9"]}]},
                {"ts":"1704070100.000200","user":"U2","text":"reply &amp; *more* ~old~ 2*3*4","thread_ts":"1704070000.000100"},
                {"ts":"1704070200.000300","user":"U1","subtype":"channel_join","text":"<@U1> has joined"},
                {"ts":"1704070300.000400","user":"U1","text":"",
                 "files":[{"id":"F1","name":"notes.txt","mimetype":"text/plain","size":10,"url_private_download":format!("{}/files/F1/notes.txt", files.uri())}]},
                {"ts":"1704070400.000500","bot_id":"B1","subtype":"bot_message","username":"deploybot","text":"deployed"},
                {"ts":"1704070500.000600","user":"U2","text":"shared again",
                 "files":[{"id":"F1","name":"notes.txt","mimetype":"text/plain","size":10,"url_private_download":format!("{}/files/F1/notes.txt", files.uri())},
                          {"id":"F3","name":"broken.txt","mimetype":"text/plain","size":5,"url_private_download":format!("{}/files/F3/broken.txt", files.uri())}]}
            ]),
        ),
        (
            format!("{random}/2024-01-02.json"),
            json!([{"ts":"1704160000.000100","user":"U1","text":"in random"}]),
        ),
    ]);

    let job = upload(&state, &app, &admin, &zip).await?;
    assert_eq!(job["team_id"], team.as_str());
    assert_eq!(job["total_messages"], 6);
    let channels = job["analysis"]["channels"].as_array().unwrap();
    // general は予約語なので名前を変える提案になる
    assert_eq!(channels[0]["suggested_name"], "slack-general");
    assert_eq!(channels[0]["message_count"], 5);
    let id = job["id"].as_str().unwrap().to_string();
    let general_name = format!("slack-general-{suffix}");

    // 予約語のチャンネル名は拒否する
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{id}/start"),
        &admin,
        Some(json!({"mappings":[{"slack_channel_id":"C1","action":"create","name":"general"}]})),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, started) = start(
        &app,
        &admin,
        &id,
        json!([
            {"slack_channel_id":"C1","action":"create","name":general_name},
            {"slack_channel_id":"C2","action":"existing","channel_id":existing}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["source_id"], id.as_str());
    let run = started["id"].as_str().unwrap().to_string();
    let done = drive(&state, &app, &admin, &run, "completed").await?;
    assert_eq!(done["processed_messages"], 6);
    assert_eq!(done["total_files"], 3);
    assert_eq!(done["processed_files"], 3);
    assert_eq!(done["pending_files"], 0);
    // F3 は取得に失敗したまま完了し、管理者が取得し直せる
    assert_eq!(done["failed_files"], 1, "{done}");
    // 取得し直せる添付が残るジョブは、新しい履歴が増えても一覧から押し出されない
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement(
            "INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id,source_id) \
             SELECT 'history-'||g,created_by,'completed',file_name,file_size,storage_key,team_id,source_id \
             FROM slack_import_jobs, generate_series(1,60) g WHERE id=$1",
        )
        .bind(run.as_str())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }
    let (status, listed) = request(&app, "GET", "/api/admin/slack-imports", &admin, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listed["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["id"] == run.as_str()),
        "a job with files to fetch again stays listed"
    );
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("DELETE FROM slack_import_jobs WHERE id LIKE 'history-%'")
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
    }
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{run}/fetch-files"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let done = drive(&state, &app, &admin, &run, "completed").await?;
    assert_eq!(done["failed_files"], 0, "{done}");

    let general = backend::slack::channel_id(&team, "C1");
    let messages = sql(
        &state,
        "SELECT json_build_object('id',m.id,'content',m.content,'parent_id',m.parent_id,\
         'created_at',m.created_at,'user_name',u.user_name,'deactivated',u.is_deactivated,'bot',u.is_bot)::text \
         FROM messages m JOIN users u ON u.id=m.user_id WHERE m.channel_id=$1 ORDER BY m.created_at",
        &[&general],
    )
    .await?;
    assert_eq!(messages.len(), 5);
    let root = &messages[0];
    assert_eq!(
        root["id"],
        backend::slack::message_id(&team, "C1", "1704070000.000100")
    );
    assert_eq!(root["parent_id"], Value::Null);
    assert_eq!(root["deactivated"], true);
    assert_eq!(root["created_at"], "2024-01-01T00:46:40");
    // 未連携のメンバーもプレースホルダとして参加させ、連携時に本人へ移せるようにする
    let placeholders = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM channel_user WHERE channel_id=$1 AND user_id IN ($2,$3)",
        &[
            &general,
            &backend::slack::user_id(&team, "U1"),
            &backend::slack::user_id(&team, "U2"),
        ],
    )
    .await?;
    assert_eq!(placeholders[0]["n"], 2);
    // U2 の Slack 名 admin は予約語なので別名になる
    let bob = messages[1]["user_name"].as_str().unwrap().to_string();
    assert_ne!(bob, "admin");
    let existing_name = sql(
        &state,
        "SELECT json_build_object('n',channel_name)::text FROM channels WHERE id=$1",
        &[&existing],
    )
    .await?[0]["n"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        root["content"],
        format!("hello @{bob} see #{existing_name}")
    );
    assert_eq!(messages[1]["parent_id"], root["id"]);
    // Slack の書式は MFM になり、書式でない記号 (`2*3*4`) はそのまま残る
    assert_eq!(messages[1]["content"], "reply & **more** ~~old~~ 2*3*4");
    assert_eq!(messages[3]["bot"], true);
    assert_eq!(messages[3]["content"], "deployed");

    let reactions = sql(
        &state,
        "SELECT json_build_object('emoji',emoji)::text FROM message_reactions WHERE message_id=$1",
        &[root["id"].as_str().unwrap()],
    )
    .await?;
    assert_eq!(reactions.len(), 2);
    assert!(reactions.iter().all(|r| r["emoji"] == "👍"));

    let attachments = sql(
        &state,
        "SELECT json_build_object('name',file_name,'size',file_size,'key',storage_key)::text \
         FROM attachments WHERE message_id=$1",
        &[messages[2]["id"].as_str().unwrap()],
    )
    .await?;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["name"], "notes.txt");
    // 取得する添付は添付として表示するので、本文にファイル名を入れない
    assert_eq!(messages[2]["content"], "");
    // 同じ F1 を共有する別のメッセージにも、それぞれ添付が付く
    let shared = sql(
        &state,
        "SELECT json_build_object('name',file_name)::text FROM attachments WHERE message_id=$1 ORDER BY file_name",
        &[messages[4]["id"].as_str().unwrap()],
    )
    .await?;
    assert_eq!(
        shared,
        vec![json!({"name": "broken.txt"}), json!({"name": "notes.txt"})]
    );
    assert_eq!(attachments[0]["size"], 10);
    let stored = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        Ok::<_, anyhow::Error>(
            state
                .s3_client
                .get_object()
                .bucket(&config.s3.bucket)
                .key(attachments[0]["key"].as_str().unwrap())
                .send()
                .await?
                .body
                .collect()
                .await?
                .into_bytes(),
        )
    })
    .await
    .map_err(|_| anyhow::anyhow!("S3 GetObject timed out after 10s"))??;
    assert_eq!(&stored[..], b"hello file");

    // 既存チャンネルにも取り込まれ、管理者の既読位置が最新になる
    let in_existing = sql(
        &state,
        "SELECT json_build_object('content',content)::text FROM messages WHERE channel_id=$1",
        &[&existing],
    )
    .await?;
    assert_eq!(in_existing.len(), 2);
    let read = sql(
        &state,
        "SELECT json_build_object('at',last_read_at)::text FROM channel_read_states WHERE user_id=$1 AND channel_id=$2",
        &[&admin, &existing],
    )
    .await?;
    // 取り込んだ過去ログの最新までで止め、2026 年の通常の投稿は未読のまま残す
    assert_eq!(read[0]["at"], "2024-01-02T01:46:40");
    let cursor = sql(
        &state,
        "SELECT json_build_object('ts',last_ts)::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id='C1'",
        &[&team],
    )
    .await?;
    assert_eq!(cursor[0]["ts"], "1704070500.000600");

    // エクスポートと ZIP は残り、続けて別のチャンネルも取り込める
    assert_s3_object_exists(
        &state.s3_client,
        &config.s3.bucket,
        &format!("slack-imports/{id}/export.zip"),
        true,
    )
    .await?;
    let (_, export) = request(
        &app,
        "GET",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(export["status"], "ready");
    assert_eq!(
        export["analysis"]["channels"][0]["existing_channel"]["id"],
        general.as_str()
    );

    // 失敗したジョブに残った添付を想定し、1 件を取得失敗の状態に戻しておく
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement(
            "UPDATE slack_files SET status='failed',attachment_id=NULL WHERE team_id=$1 AND slack_file_id='F3'",
        )
        .bind(team.as_str())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    // 同じエクスポートをもう一度取り込んでも重複しない。新しい ZIP は古いエクスポートを置き換える
    let again = upload(&state, &app, &admin, &zip).await?;
    let (_, old) = request(
        &app,
        "GET",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(old["status"], "cancelled");
    assert_s3_object_exists(
        &state.s3_client,
        &config.s3.bucket,
        &format!("slack-imports/{id}/export.zip"),
        false,
    )
    .await?;
    let (status, _) = start(
        &app,
        &admin,
        &id,
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":general}]),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    let channels = again["analysis"]["channels"].as_array().unwrap();
    assert_eq!(channels[0]["existing_channel"]["id"], general.as_str());
    assert_eq!(channels[1]["existing_channel"]["id"], existing.as_str());
    let again_id = again["id"].as_str().unwrap().to_string();
    // 以前と別の取り込み先にはできない (過去ログが移らないため)
    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{again_id}/start"),
        &admin,
        Some(json!({"mappings":[
            {"slack_channel_id":"C2","action":"create","name":format!("moved-{suffix}")}
        ]})),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, again_run) = start(
        &app,
        &admin,
        &again_id,
        json!([
            {"slack_channel_id":"C1","action":"existing","channel_id":general},
            {"slack_channel_id":"C2","action":"skip"}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let again_id = again_run["id"].as_str().unwrap().to_string();
    let again_done = drive(&state, &app, &admin, &again_id, "completed").await?;
    // 取得に失敗していた添付は新しいジョブに引き継がれ、取得し直される
    let handed = sql(
        &state,
        "SELECT json_build_object('job_id',job_id,'status',status)::text FROM slack_files WHERE team_id=$1 AND slack_file_id='F3'",
        &[&team],
    )
    .await?;
    assert_eq!(handed[0], json!({"job_id": again_id, "status": "done"}));
    assert_eq!(again_done["processed_files"], 1, "{again_done}");
    let count = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1",
        &[&general],
    )
    .await?;
    assert_eq!(count[0]["n"], 5);

    // 取り込み先を削除した後は、過去ログが残っているので取り込み直せない
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("UPDATE channels SET deleted_at=CURRENT_TIMESTAMP WHERE id=$1")
            .bind(general.as_str())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
    }
    let third = upload(&state, &app, &admin, &zip).await?;
    let general_entry = &third["analysis"]["channels"][0];
    assert_eq!(general_entry["existing_channel"], Value::Null);
    assert_eq!(general_entry["deleted_channel"], general_name.as_str());
    let third_id = third["id"].as_str().unwrap().to_string();
    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{third_id}/start"),
        &admin,
        Some(json!({"mappings":[
            {"slack_channel_id":"C1","action":"create","name":format!("again-{suffix}")}
        ]})),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    Ok(())
}

#[tokio::test]
async fn test_slack_zip_preserves_message_username_for_pre_resolved_mentions_and_reactions()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    ensure_bucket_exists(&state.s3_client, &state.config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_fallback_admin").await?;
    let role_result: Result<(), toasty::Error> = async {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin"
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(error) = role_result {
        let mut cleanup_db = state.db.clone();
        let mut cleanup_tx = cleanup_db.transaction().await?;
        exec(
            &mut cleanup_tx,
            "DELETE FROM user_role WHERE user_id=$1",
            &[&admin],
        )
        .await?;
        exec(&mut cleanup_tx, "DELETE FROM users WHERE id=$1", &[&admin]).await?;
        cleanup_tx.commit().await?;
        return Err(error.into());
    }
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let channel_name = format!("fallback-{}", uuid::Uuid::new_v4().simple());
    let mut run_id = None;
    let mut export_id = None;
    let test_result: anyhow::Result<()> = async {
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U-SEED","team_id":team,"name":"seed"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C-FALLBACK","name":"fallback","created":1704067200,"members":["U-FALLBACK"]} ]),
        ),
        (
            "fallback/2024-01-01.json".into(),
            json!([
                {"ts":"1704070000.000100","user":"U-FALLBACK","username":"deploybot","bot_profile":{"name":"lower-priority"},"text":"posted"},
                {"ts":"1704070050.000150","bot_id":"B-BOT","subtype":"bot_message","bot_profile":{"name":"profilebot"},"text":"bot posted"},
                {"ts":"1704070100.000200","user":"U-OTHER","text":"<@U-FALLBACK>","reactions":[{"name":"+1","users":["U-FALLBACK"]}]}
            ]),
        ),
    ]);
    let job = upload(&state, &app, &admin, &zip).await?;
    anyhow::ensure!(job["team_id"] == team.as_str(), "uploaded ZIP team mismatch");
    let export = job["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("export job has no id: {job}"))?;
    export_id = Some(export.to_string());
    let (status, started) = start(&app, &admin, export, json!([{"slack_channel_id":"C-FALLBACK","action":"create","name":channel_name}])).await?;
    anyhow::ensure!(status == StatusCode::OK, "failed to start import: {started}");
    let run = started["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("import job has no id: {started}"))?;
    run_id = Some(run.to_string());
    let done = drive_checked(&state, &app, &admin, run, "completed").await?;
    anyhow::ensure!(done["processed_messages"] == 3, "unexpected processed count: {done}");
    let placeholder = backend::slack::user_id(&team, "U-FALLBACK");
    let users = sql(&state, "SELECT json_build_object('user_name',user_name,'display_name',display_name)::text FROM users WHERE id=$1", &[&placeholder]).await?;
    anyhow::ensure!(users.len() == 1, "expected one fallback placeholder");
    let fallback_user_name = users[0]["user_name"].as_str().unwrap();
    anyhow::ensure!(fallback_user_name.starts_with("slack-deploybot"), "unexpected fallback username: {fallback_user_name}");
    anyhow::ensure!(users[0]["display_name"] == "deploybot", "unexpected fallback display name");
    let bot_placeholder = backend::slack::user_id(&team, "B-BOT");
    let bot_users = sql(&state, "SELECT json_build_object('user_name',user_name,'display_name',display_name)::text FROM users WHERE id=$1", &[&bot_placeholder]).await?;
    anyhow::ensure!(bot_users.len() == 1, "expected one bot placeholder");
    anyhow::ensure!(bot_users[0]["user_name"].as_str().unwrap().starts_with("slack-profilebot"), "unexpected bot username");
    anyhow::ensure!(bot_users[0]["display_name"] == "profilebot", "unexpected bot display name");
    let reactions = sql(
        &state,
        "SELECT json_build_object('count',count(*))::text FROM message_reactions WHERE user_id=$1",
        &[&placeholder],
    )
    .await?;
    anyhow::ensure!(reactions[0]["count"] == 1, "expected one imported reaction");
    let other = backend::slack::user_id(&team, "U-OTHER");
    let mention_pattern = format!("%{fallback_user_name}%");
    let mentions = sql(&state, "SELECT json_build_object('content',content)::text FROM messages WHERE user_id=$1 AND content LIKE $2", &[&other, &mention_pattern]).await?;
    anyhow::ensure!(mentions.len() == 1, "mention rendering must use the same resolved fallback");
    Ok(())
    }
    .await;
    // This integration test uses the shared backend test database. Remove every row
    // committed by the import, including the message author U-OTHER, after asserting.
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    exec(&mut tx, "SET LOCAL zoo.suppress_app_events = 'on'", &[]).await?;
    exec(
        &mut tx,
        "DELETE FROM messages WHERE channel_id IN (SELECT id FROM channels WHERE channel_name=$1)",
        &[&channel_name],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM channel_user WHERE channel_id IN (SELECT id FROM channels WHERE channel_name=$1)",
        &[&channel_name],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM app_deliveries WHERE channel_id IN (SELECT id FROM channels WHERE channel_name=$1)",
        &[&channel_name],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM slack_channels WHERE team_id=$1",
        &[&team],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM channels WHERE channel_name=$1",
        &[&channel_name],
    )
    .await?;
    exec(
        &mut tx,
        "DELETE FROM slack_users WHERE team_id=$1",
        &[&team],
    )
    .await?;
    let fallback_user = backend::slack::user_id(&team, "U-FALLBACK");
    let bot_user = backend::slack::user_id(&team, "B-BOT");
    let other_user = backend::slack::user_id(&team, "U-OTHER");
    exec(
        &mut tx,
        "DELETE FROM users WHERE id IN ($1,$2,$3)",
        &[&fallback_user, &bot_user, &other_user],
    )
    .await?;
    if let Some(run) = run_id.as_deref() {
        exec(&mut tx, "DELETE FROM slack_import_jobs WHERE id=$1", &[run]).await?;
    }
    if let Some(export) = export_id.as_deref() {
        let key = format!("slack-imports/{export}/export.zip");
        let _ = state
            .s3_client
            .delete_object()
            .bucket(&state.config.s3.bucket)
            .key(key)
            .send()
            .await;
        exec(
            &mut tx,
            "DELETE FROM slack_import_jobs WHERE id=$1",
            &[export],
        )
        .await?;
    }
    exec(&mut tx, "DELETE FROM user_role WHERE user_id=$1", &[&admin]).await?;
    exec(&mut tx, "DELETE FROM users WHERE id=$1", &[&admin]).await?;
    tx.commit().await?;
    test_result
}

#[derive(Clone, Copy)]
enum ZipTargetRace {
    Public,
    Deleted,
    Encrypted,
    Remapped,
}

#[derive(Clone, Copy)]
enum TestBodyFailure {
    None,
    Panic,
    EmptyResult,
}

#[derive(Clone, Copy)]
enum CleanupFailure {
    None,
    DatabaseError,
    S3Error,
    DatabaseTimeout,
    S3Timeout,
    JobDeleteError,
}

async fn run_private_zip_visibility_race(
    change: ZipTargetRace,
    inject_transient_failure: bool,
    inject_worker_timeout: bool,
    body_failure: TestBodyFailure,
    cleanup_failure: CleanupFailure,
    inject_update_lock_timeout: bool,
) -> anyhow::Result<()> {
    let database_url = std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    let mut config = Config::from_env();
    config.database_url = database_url.clone();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let admin = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    let remapped_target = uuid::Uuid::new_v4().to_string();
    let slack_member = backend::slack::user_id(&team, "U-PRIVATE");
    let slack_late_author = backend::slack::user_id(&team, "U-LATE");
    let channel_name = format!("private-race-{suffix}");
    let function = format!("slack_import_gate_{suffix}");
    let trigger = format!("slack_import_gate_trg_{suffix}");
    let sequence = format!("slack_import_gate_seq_{suffix}");
    let lock_key = uuid::Uuid::new_v4().as_u128() as i64;
    let (connection, driver) =
        tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    connection
        .query_one("SELECT pg_advisory_lock($1)", &[&lock_key])
        .await?;

    let mut export_id = None;
    let mut run_id = None;
    let mut worker_handle = None;
    let caught_result = std::panic::AssertUnwindSafe(async {
        {
            let mut tx = db.clone();
            let mut tx = tx.transaction().await?;
            exec(&mut tx, "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,$2)", &[&admin, &format!("race-admin-{suffix}")]).await?;
            exec(&mut tx, "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin')", &[&admin]).await?;
            exec(&mut tx, "INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,$2,$3,true,false)", &[&target, &channel_name, &admin]).await?;
            exec(&mut tx, "INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,$2,$3,true,false)", &[&remapped_target, &format!("{channel_name}-remapped"), &admin]).await?;
            exec(&mut tx, "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())", &[&admin, &target]).await?;
            tx.commit().await?;
        }
        let zip = build_zip(&[
            ("users.json".into(), json!([{"id":"U-PRIVATE","team_id":team,"name":"private-user"}])),
            ("channels.json".into(), json!([])),
            ("groups.json".into(), json!([{"id":"C-PRIVATE","name":"private","created":1704067200,"members":["U-PRIVATE"]}])),
            ("private/2024-01-01.json".into(), json!([{"ts":"1704070000.000100","user":"U-PRIVATE","text":"first private day"}])),
            ("private/2024-01-02.json".into(), json!([{"ts":"1704156400.000100","user":"U-LATE","text":"second private day"}])),
        ]);
        let export = upload(&state, &app, &admin, &zip).await?;
        let source_id = export["id"].as_str().ok_or_else(|| anyhow::anyhow!("missing export id: {export}"))?.to_string();
        export_id = Some(source_id.clone());
        let (status, started) = start(&app, &admin, &source_id, json!([{"slack_channel_id":"C-PRIVATE","action":"existing","channel_id":target}])).await?;
        anyhow::ensure!(status == StatusCode::OK, "cannot start import: {started}");
        let run = started["id"].as_str().ok_or_else(|| anyhow::anyhow!("missing run id: {started}"))?.to_string();
        run_id = Some(run.clone());

        // Pause the second day's placeholder insert while the target changes. The
        // regression mode returns normally here, so only the save-transaction guard
        // can reject the changed destination.
        if inject_transient_failure {
            connection.batch_execute(&format!("CREATE SEQUENCE {sequence}")).await?;
        }
        let failure = if inject_transient_failure {
            format!("IF nextval('{sequence}') = 1 THEN RAISE EXCEPTION 'injected one-time transient Slack import failure'; END IF;")
        } else {
            String::new()
        };
        let gate = format!(
            "CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id = '{slack_late_author}' THEN PERFORM pg_advisory_xact_lock({lock_key}); {failure} END IF; RETURN NEW; END $$; CREATE TRIGGER {trigger} BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION {function}()"
        );
        connection.batch_execute(&gate).await?;
        let worker_state = state.clone();
        worker_handle = Some(tokio::spawn(async move {
            backend::slack::worker::run_once(&worker_state).await
        }));

        let mut waiting = false;
        for _ in 0..300 {
            let row = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                connection.query_one(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND wait_event_type='Lock' AND query ILIKE '%INSERT INTO users%')",
                    &[],
                ),
            )
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for the ZIP worker test gate"))??;
            if row.get::<_, bool>(0) {
                waiting = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        if !waiting {
            let status = sql(
                &state,
                "SELECT json_build_object('status',status,'error',error)::text FROM slack_import_jobs WHERE id=$1",
                &[&run],
            )
            .await?;
            anyhow::bail!("ZIP import did not reach the gated placeholder insert; job={status:?}, worker_finished={}", worker_handle.as_ref().is_some_and(tokio::task::JoinHandle::is_finished));
        }

        match body_failure {
            TestBodyFailure::None => {}
            TestBodyFailure::Panic => panic!("injected ZIP test body panic"),
            TestBodyFailure::EmptyResult => {
                let empty: Vec<Value> = Vec::new();
                empty
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("injected empty ZIP test result"))?;
            }
        }

        if inject_worker_timeout {
            let mut worker = worker_handle
                .take()
                .ok_or_else(|| anyhow::anyhow!("ZIP worker handle missing"))?;
            if tokio::time::timeout(std::time::Duration::from_millis(100), &mut worker)
                .await
                .is_ok()
            {
                anyhow::bail!("injected ZIP worker did not remain blocked on the test gate");
            }
            worker.abort();
            match tokio::time::timeout(std::time::Duration::from_secs(5), worker).await {
                Ok(Err(error)) if error.is_cancelled() => {}
                Ok(Ok(_)) => anyhow::bail!("injected ZIP worker completed instead of timing out"),
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => anyhow::bail!("injected ZIP worker did not stop after abort"),
            }
            anyhow::bail!("injected ZIP worker timeout while held at test gate");
        }

        if inject_update_lock_timeout {
            connection.batch_execute("BEGIN").await?;
            connection
                .query_one(
                    "SELECT id FROM channels WHERE id=$1 FOR UPDATE",
                    &[&target],
                )
                .await?;
        }

        // Day one committed while the target was private. Pause resolution for day two,
        // then make the selected target public or delete it before that day's save check.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut tx_db = state.db.clone();
            let mut tx = tx_db.transaction().await?;
            match change {
                ZipTargetRace::Public => exec(&mut tx, "UPDATE channels SET is_private=false WHERE id=$1", &[&target]).await?,
                ZipTargetRace::Deleted => exec(&mut tx, "UPDATE channels SET deleted_at=CURRENT_TIMESTAMP WHERE id=$1", &[&target]).await?,
                ZipTargetRace::Encrypted => exec(&mut tx, "UPDATE channels SET encrypted=true WHERE id=$1", &[&target]).await?,
                ZipTargetRace::Remapped => exec(&mut tx, "UPDATE slack_channels SET channel_id=$3 WHERE team_id=$1 AND slack_channel_id=$2", &[&team, "C-PRIVATE", &remapped_target]).await?,
            }
            tx.commit().await?;
            anyhow::Ok(())
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out after 5s changing the ZIP destination"))??;
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connection.query_one("SELECT pg_advisory_unlock($1)", &[&lock_key]),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out after 5s releasing the ZIP test gate"))??;
        let mut worker = worker_handle
            .take()
            .ok_or_else(|| anyhow::anyhow!("ZIP worker handle missing"))?;
        match tokio::time::timeout(std::time::Duration::from_secs(15), &mut worker).await {
            Ok(joined) => {
                joined??;
            }
            Err(_) => {
                worker.abort();
                match tokio::time::timeout(std::time::Duration::from_secs(5), worker)
                    .await
                    .map_err(|_| anyhow::anyhow!("ZIP worker did not stop after abort"))?
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => return Err(anyhow::anyhow!("ZIP worker failed after abort: {error}")),
                    Err(error) if error.is_cancelled() => {}
                    Err(error) => return Err(error.into()),
                }
                anyhow::bail!("ZIP worker timed out after releasing the test gate");
            }
        }

        let rows = sql(&state, "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1", &[&target]).await?;
        anyhow::ensure!(rows[0]["n"] == 1, "later private ZIP data was saved after target state changed: {rows:?}");
        let later = sql(&state, "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND content='second private day'", &[&target]).await?;
        anyhow::ensure!(later[0]["n"] == 0, "second day's private content must not be saved: {later:?}");
        let (_, job) = request(&app, "GET", &format!("/api/admin/slack-imports/{run}"), &admin, None).await?;
        anyhow::ensure!(job["error"].is_string(), "changed target should stop the ZIP job with an error: {job}");

        if !inject_transient_failure {
            let state_after_guard = sql(
                &state,
                "SELECT json_build_object('status',status,'error',error,'lease_token',lease_token,'progress',import_progress)::text FROM slack_import_jobs WHERE id=$1",
                &[&run],
            ).await?;
            let guarded = &state_after_guard[0];
            anyhow::ensure!(guarded["lease_token"].is_null(), "guard rejection must release the worker lease: {guarded:?}");
            let progress: serde_json::Map<String, Value> = serde_json::from_str(
                guarded["progress"].as_str().ok_or_else(|| anyhow::anyhow!("missing progress after guard rejection: {guarded:?}"))?,
            )?;
            anyhow::ensure!(progress.get("C-PRIVATE").and_then(Value::as_str) == Some("private/2024-01-01.json"), "second day advanced import progress: {guarded:?}");
            match change {
                ZipTargetRace::Public => anyhow::ensure!(guarded["status"] == "failed" && guarded["error"].as_str().is_some_and(|e| e.contains("private channel")), "public target did not fail in the save guard: {guarded:?}"),
                ZipTargetRace::Deleted => anyhow::ensure!(guarded["status"] == "failed" && guarded["error"].as_str().is_some_and(|e| e.contains("no longer exists")), "deleted target did not fail in the save guard: {guarded:?}"),
                ZipTargetRace::Encrypted => anyhow::ensure!(guarded["status"] == "importing" && guarded["error"].as_str().is_some_and(|e| e.contains("E2EE_PLAINTEXT_FORBIDDEN")), "encrypted target did not fail in the save guard: {guarded:?}"),
                ZipTargetRace::Remapped => anyhow::ensure!(guarded["status"] == "importing" && guarded["error"].as_str().is_some_and(|e| e.contains("no longer maps")), "remapped target did not fail in the save guard: {guarded:?}"),
            }
            let late_user = sql(&state, "SELECT json_build_object('n',count(*))::text FROM users WHERE id=$1", &[&slack_late_author]).await?;
            anyhow::ensure!(late_user[0]["n"] == 0, "guard failure must roll back the second day's placeholder: {late_user:?}");
            let late_messages = sql(&state, "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND content='second private day'", &[&target]).await?;
            anyhow::ensure!(late_messages[0]["n"] == 0, "guard failure persisted second-day ZIP content: {late_messages:?}");
            let remapped_messages = sql(&state, "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND content='second private day'", &[&remapped_target]).await?;
            anyhow::ensure!(remapped_messages[0]["n"] == 0, "guard failure persisted second-day ZIP content in the remapped destination: {remapped_messages:?}");
            return Ok(());
        }

        // The gate caused a real transient database error. Prove fail() left this job
        // eligible for retry rather than mistaking the subsequent BadRequest for a retry.
        let transient = sql(
            &state,
            "SELECT json_build_object('status',status,'attempts',attempts,'error',error,'lease_token',lease_token,'retry_scheduled',available_at>now(),'progress',import_progress)::text FROM slack_import_jobs WHERE id=$1",
            &[&run],
        ).await?;
        anyhow::ensure!(transient[0]["status"] == "importing", "transient error must leave the import retryable: {transient:?}");
        anyhow::ensure!(transient[0]["error"].as_str().is_some_and(|e| e.contains("injected one-time transient Slack import failure")), "expected the injected transient DB error: {transient:?}");
        anyhow::ensure!(transient[0]["lease_token"].is_null(), "failed attempt must release its lease: {transient:?}");
        anyhow::ensure!(transient[0]["retry_scheduled"] == true, "transient failure must schedule a future retry: {transient:?}");
        let transient_attempts = transient[0]["attempts"].as_i64().ok_or_else(|| anyhow::anyhow!("missing attempts: {transient:?}"))?;
        let transient_progress: serde_json::Map<String, Value> = serde_json::from_str(
            transient[0]["progress"].as_str().ok_or_else(|| anyhow::anyhow!("missing import progress: {transient:?}"))?,
        )?;
        anyhow::ensure!(transient_progress.get("C-PRIVATE").and_then(Value::as_str) == Some("private/2024-01-01.json"), "first day should be committed before transient failure: {transient:?}");

        // Make the transiently failed job eligible and prove run_once actually reacquires
        // it. That attempt must then reject the changed target before saving day two.
        let mut retry_db = state.db.clone();
        let mut retry_tx = retry_db.transaction().await?;
        exec(&mut retry_tx, "UPDATE slack_import_jobs SET available_at=now() WHERE id=$1", &[&run]).await?;
        retry_tx.commit().await?;
        let eligible = sql(&state, "SELECT json_build_object('eligible',status='importing' AND available_at<=now() AND lease_token IS NULL)::text FROM slack_import_jobs WHERE id=$1", &[&run]).await?;
        anyhow::ensure!(eligible[0]["eligible"] == true, "transient job was not made eligible before run_once: {eligible:?}");
        anyhow::ensure!(backend::slack::worker::run_once(&state).await?, "eligible transiently failed job was not reacquired");
        let retried = sql(
            &state,
            "SELECT json_build_object('status',status,'attempts',attempts,'error',error,'finished_at',finished_at,'progress',import_progress)::text FROM slack_import_jobs WHERE id=$1",
            &[&run],
        ).await?;
        anyhow::ensure!(retried[0]["attempts"] == transient_attempts + 1, "retry must increment attempts exactly once: before={transient_attempts}, after={retried:?}");
        let retried_progress: serde_json::Map<String, Value> = serde_json::from_str(
            retried[0]["progress"].as_str().ok_or_else(|| anyhow::anyhow!("missing retry progress: {retried:?}"))?,
        )?;
        anyhow::ensure!(retried_progress == transient_progress, "rejected second day must not advance import progress: {retried:?}");
        match change {
            ZipTargetRace::Public => {
                anyhow::ensure!(retried[0]["status"] == "failed" && retried[0]["finished_at"].is_string(), "public-target BadRequest must permanently fail: {retried:?}");
                anyhow::ensure!(retried[0]["error"].as_str().is_some_and(|e| e.contains("private channel")), "public target should be rejected as non-private: {retried:?}");
            }
            ZipTargetRace::Deleted => {
                anyhow::ensure!(retried[0]["status"] == "failed" && retried[0]["finished_at"].is_string(), "deleted-target BadRequest must permanently fail: {retried:?}");
                anyhow::ensure!(retried[0]["error"].as_str().is_some_and(|e| e.contains("deleted")), "deleted target should be rejected through its existing mapping: {retried:?}");
            }
            ZipTargetRace::Encrypted => {
                // The guard returns a coded E2EE error, which the worker keeps retryable;
                // record the real state rather than calling it a permanent BadRequest.
                anyhow::ensure!(retried[0]["status"] == "importing" && retried[0]["finished_at"].is_null(), "encrypted-target guard should retain its actual retryable state: {retried:?}");
                anyhow::ensure!(retried[0]["error"].as_str().is_some_and(|e| e.contains("E2EE_PLAINTEXT_FORBIDDEN")), "encrypted target should be rejected by the plaintext guard: {retried:?}");
            }
            ZipTargetRace::Remapped => {
                anyhow::ensure!(retried[0]["status"] == "failed" && retried[0]["finished_at"].is_string(), "remapped-target BadRequest must permanently fail: {retried:?}");
                anyhow::ensure!(retried[0]["error"].as_str().is_some_and(|e| e.contains("already imported")), "remapped target should be rejected by the existing-target guard: {retried:?}");
            }
        }
        let actual_target = sql(
            &state,
            "SELECT json_build_object('private',is_private,'encrypted',encrypted,'deleted',deleted_at IS NOT NULL)::text FROM channels WHERE id=$1",
            &[&target],
        ).await?;
        match change {
            ZipTargetRace::Public => anyhow::ensure!(actual_target[0]["private"] == false && actual_target[0]["encrypted"] == false && actual_target[0]["deleted"] == false, "public case did not actually make the target public: {actual_target:?}"),
            ZipTargetRace::Deleted => anyhow::ensure!(actual_target[0]["private"] == true && actual_target[0]["deleted"] == true, "deleted case did not actually delete the target: {actual_target:?}"),
            ZipTargetRace::Encrypted => anyhow::ensure!(actual_target[0]["private"] == true && actual_target[0]["encrypted"] == true && actual_target[0]["deleted"] == false, "encrypted case did not actually encrypt the target: {actual_target:?}"),
            ZipTargetRace::Remapped => {
                anyhow::ensure!(actual_target[0]["private"] == true && actual_target[0]["encrypted"] == false && actual_target[0]["deleted"] == false, "remapped source target should remain a live plaintext private channel: {actual_target:?}");
                let mapping = sql(&state, "SELECT json_build_object('channel_id',channel_id)::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id='C-PRIVATE'", &[&team]).await?;
                anyhow::ensure!(mapping.len() == 1 && mapping[0]["channel_id"] == remapped_target, "Slack mapping was not actually remapped: {mapping:?}");
            }
        }
        let retry_messages = sql(&state, "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1", &[&target]).await?;
        anyhow::ensure!(retry_messages[0]["n"] == 1, "retry saved later private content: {retry_messages:?}");
        let retry_later = sql(&state, "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND content='second private day'", &[&target]).await?;
        anyhow::ensure!(retry_later[0]["n"] == 0, "retry saved the later private ZIP post: {retry_later:?}");
        if matches!(change, ZipTargetRace::Public | ZipTargetRace::Deleted) {
            let mut permanent_db = state.db.clone();
            let mut permanent_tx = permanent_db.transaction().await?;
            exec(&mut permanent_tx, "UPDATE slack_import_jobs SET available_at=now() WHERE id=$1", &[&run]).await?;
            permanent_tx.commit().await?;
            anyhow::ensure!(!backend::slack::worker::run_once(&state).await?, "permanently failed public/deleted job was claimed again");
            let unchanged = sql(&state, "SELECT json_build_object('status',status,'attempts',attempts)::text FROM slack_import_jobs WHERE id=$1", &[&run]).await?;
            anyhow::ensure!(unchanged[0]["status"] == "failed" && unchanged[0]["attempts"] == transient_attempts + 1, "public/deleted job changed after a subsequent worker pass: {unchanged:?}");
        }
        Ok(())
    })
    .catch_unwind()
    .await;
    let result = match caught_result {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            Err(anyhow::anyhow!("test body panicked: {message}"))
        }
    };

    let mut cleanup_errors = Vec::new();
    macro_rules! attempt_cleanup {
        ($label:expr, $future:expr) => {
            match tokio::time::timeout(std::time::Duration::from_secs(5), $future).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => cleanup_errors.push(format!("{}: {error:#}", $label)),
                Err(_) => cleanup_errors.push(format!("{}: timed out after 5s", $label)),
            }
        };
    }

    if let Some(worker) = worker_handle {
        worker.abort();
        attempt_cleanup!("await aborted ZIP worker", async {
            match worker.await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(anyhow::anyhow!("ZIP worker failed during cleanup: {error}")),
                Err(error) if error.is_cancelled() => Ok(()),
                Err(error) => Err(anyhow::Error::from(error)),
            }
        });
    }
    if inject_update_lock_timeout {
        attempt_cleanup!("rollback target lock injection", async {
            connection.batch_execute("ROLLBACK").await?;
            Ok::<(), anyhow::Error>(())
        });
    }
    attempt_cleanup!("release ZIP test gate", async {
        connection
            .query_one("SELECT pg_advisory_unlock($1)", &[&lock_key])
            .await?;
        Ok::<(), anyhow::Error>(())
    });
    attempt_cleanup!("drop test trigger", async {
        connection
            .batch_execute(&format!("DROP TRIGGER IF EXISTS {trigger} ON users"))
            .await?;
        Ok::<(), anyhow::Error>(())
    });
    attempt_cleanup!("drop test function", async {
        connection
            .batch_execute(&format!("DROP FUNCTION IF EXISTS {function}()"))
            .await?;
        Ok::<(), anyhow::Error>(())
    });
    attempt_cleanup!("drop test sequence", async {
        connection
            .batch_execute(&format!("DROP SEQUENCE IF EXISTS {sequence}"))
            .await?;
        Ok::<(), anyhow::Error>(())
    });
    let mut export_object_removal_confirmed = false;
    if let Some(export) = export_id.as_deref() {
        // The import run and export job share the export's storage_key. Check and
        // remove that one object before either database row is deleted.
        let key = format!("slack-imports/{export}/export.zip");
        let injection_error = match cleanup_failure {
            CleanupFailure::S3Error => {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    state
                        .s3_client
                        .delete_object()
                        .bucket("invalid bucket name")
                        .key(&key)
                        .send(),
                )
                .await;
                Some(match result {
                    Ok(Err(error)) => format!("injected S3 cleanup failure: {error}"),
                    Err(_) => "injected S3 cleanup failure: timed out after 5s".to_string(),
                    Ok(Ok(_)) => "injected S3 cleanup failure did not fail".to_string(),
                })
            }
            CleanupFailure::S3Timeout => {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::time::sleep(std::time::Duration::from_secs(6)),
                )
                .await
                {
                    Err(_) => Some("injected S3 cleanup timeout: timed out after 5s".to_string()),
                    Ok(()) => {
                        Some("injected S3 cleanup timeout injection did not time out".to_string())
                    }
                }
            }
            _ => None,
        };
        if let Some(injection_error) = injection_error {
            cleanup_errors.push(format!(
                "{injection_error}, recovery resources: job_id={export}, run_id={}, bucket={}, key={key}",
                run_id.as_deref().unwrap_or("none"),
                config.s3.bucket
            ));
        }
        let injected_s3_failure = matches!(
            cleanup_failure,
            CleanupFailure::S3Error | CleanupFailure::S3Timeout
        );
        if injected_s3_failure {
            attempt_cleanup!(format!("verify retained S3 object {key}"), async {
                assert_s3_object_exists(&state.s3_client, &config.s3.bucket, &key, true).await?;
                let run = run_id.as_deref().unwrap_or("");
                let rows = sql(&state, "SELECT json_build_object('n',count(*))::text FROM slack_import_jobs WHERE id IN ($1,$2)", &[export, run]).await?;
                let expected_jobs = if run.is_empty() { 1 } else { 2 };
                anyhow::ensure!(
                    rows.first().and_then(|row| row["n"].as_i64()) == Some(expected_jobs),
                    "export and import jobs must remain while their ZIP object exists: {rows:?}"
                );
                Ok::<(), anyhow::Error>(())
            });
            // Retry the same real RustFS object after proving the first failure left its
            // database recovery pointer intact. A failed retry keeps the job recoverable.
        }
        let removal = delete_s3_object_and_confirm(&state, &config.s3.bucket, &key).await;
        match removal {
            Ok(()) => export_object_removal_confirmed = true,
            Err(error) => cleanup_errors.push(format!(
                "remove ZIP object: {error:#}, recovery resources: job_id={export}, run_id={}, bucket={}, key={key}",
                run_id.as_deref().unwrap_or("none"), config.s3.bucket
            )),
        }
    }
    if matches!(cleanup_failure, CleanupFailure::DatabaseError) {
        attempt_cleanup!(
            "injected database cleanup failure",
            cleanup_statement(&state, "SELECT 1 / 0", &[])
        );
    }
    if matches!(cleanup_failure, CleanupFailure::DatabaseTimeout) {
        attempt_cleanup!("injected database cleanup timeout", async {
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            Ok::<(), anyhow::Error>(())
        });
    }
    let mut cleanup_db: Vec<(&str, &str, Vec<String>)> = vec![
        (
            "delete imported messages",
            "DELETE FROM messages WHERE channel_id IN ($1,$2)",
            vec![target.clone(), remapped_target.clone()],
        ),
        (
            "delete channel memberships",
            "DELETE FROM channel_user WHERE channel_id IN ($1,$2)",
            vec![target.clone(), remapped_target.clone()],
        ),
        (
            "delete app deliveries",
            "DELETE FROM app_deliveries WHERE channel_id IN ($1,$2)",
            vec![target.clone(), remapped_target.clone()],
        ),
        (
            "delete Slack channel mappings",
            "DELETE FROM slack_channels WHERE team_id=$1",
            vec![team.clone()],
        ),
        (
            "delete Slack users",
            "DELETE FROM slack_users WHERE team_id=$1",
            vec![team.clone()],
        ),
    ];
    if export_object_removal_confirmed && let Some(id) = run_id.as_deref() {
        cleanup_db.push((
            "delete import run",
            "DELETE FROM slack_import_jobs WHERE id=$1",
            vec![id.to_string()],
        ));
    }
    if export_object_removal_confirmed && let Some(id) = export_id.as_deref() {
        cleanup_db.push((
            "delete export job",
            "DELETE FROM slack_import_jobs WHERE id=$1",
            vec![id.to_string()],
        ));
    }
    cleanup_db.extend([
        (
            "delete target channel",
            "DELETE FROM channels WHERE id=$1",
            vec![target.clone()],
        ),
        (
            "delete remapped channel",
            "DELETE FROM channels WHERE id=$1",
            vec![remapped_target.clone()],
        ),
        (
            "delete admin role",
            "DELETE FROM user_role WHERE user_id=$1",
            vec![admin.clone()],
        ),
        (
            "delete test users",
            "DELETE FROM users WHERE id IN ($1,$2,$3)",
            vec![
                admin.clone(),
                slack_member.clone(),
                slack_late_author.clone(),
            ],
        ),
    ]);
    let delete_trigger = format!("slack_import_delete_failure_{suffix}");
    let delete_function = format!("slack_import_delete_failure_fn_{suffix}");
    let inject_job_delete_failure = matches!(cleanup_failure, CleanupFailure::JobDeleteError)
        && export_object_removal_confirmed;
    if inject_job_delete_failure {
        let export = export_id.as_deref().unwrap_or("");
        connection.batch_execute(&format!(
            "CREATE FUNCTION {delete_function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.id = '{export}' THEN RAISE EXCEPTION 'injected job DELETE failure'; END IF; RETURN OLD; END $$; CREATE TRIGGER {delete_trigger} BEFORE DELETE ON slack_import_jobs FOR EACH ROW EXECUTE FUNCTION {delete_function}()"
        )).await?;
    }
    for (label, statement, params) in cleanup_db {
        let refs: Vec<&str> = params.iter().map(String::as_str).collect();
        let recovery_label = if matches!(label, "delete import run" | "delete export job") {
            format!(
                "{label} (job_id={})",
                params.first().map(String::as_str).unwrap_or("unknown")
            )
        } else {
            label.to_string()
        };
        attempt_cleanup!(recovery_label, cleanup_statement(&state, statement, &refs));
    }
    if inject_job_delete_failure {
        attempt_cleanup!("drop injected job DELETE trigger", async {
            connection
                .batch_execute(&format!(
                    "DROP TRIGGER IF EXISTS {delete_trigger} ON slack_import_jobs"
                ))
                .await?;
            Ok::<(), anyhow::Error>(())
        });
        attempt_cleanup!("drop injected job DELETE function", async {
            connection
                .batch_execute(&format!("DROP FUNCTION IF EXISTS {delete_function}()"))
                .await?;
            Ok::<(), anyhow::Error>(())
        });
        attempt_cleanup!("verify failed job DELETE retained rows", async {
            let export = export_id.as_deref().unwrap_or("").to_string();
            let row = connection
                .query_one(
                    "SELECT count(*)::BIGINT FROM slack_import_jobs WHERE id=$1",
                    &[&export],
                )
                .await?;
            anyhow::ensure!(
                row.get::<_, i64>(0) == 1,
                "injected real job DELETE should leave the export row for recovery"
            );
            Ok::<(), anyhow::Error>(())
        });
        if let Some(id) = export_id.as_deref() {
            attempt_cleanup!("retry failed job DELETE after removing injection", async {
                let deleted = connection
                    .execute("DELETE FROM slack_import_jobs WHERE id=$1", &[&id])
                    .await?;
                anyhow::ensure!(
                    deleted == 1,
                    "recovery DELETE removed {deleted} export rows instead of one"
                );
                Ok::<(), anyhow::Error>(())
            });
        }
        attempt_cleanup!(
            "retry dependent test user cleanup after job DELETE recovery",
            cleanup_statement(
                &state,
                "DELETE FROM users WHERE id IN ($1,$2,$3)",
                &[&admin, &slack_member, &slack_late_author]
            )
        );
    }
    attempt_cleanup!("verify database fixture cleanup", async {
        let run = run_id.as_deref().unwrap_or("");
        let export = export_id.as_deref().unwrap_or("");
        let rows = sql(
            &state,
            "SELECT json_build_object('n', (SELECT count(*) FROM slack_import_jobs WHERE id IN ($1,$2)) + (SELECT count(*) FROM channels WHERE id IN ($3,$4)) + (SELECT count(*) FROM slack_channels WHERE team_id=$5) + (SELECT count(*) FROM slack_users WHERE team_id=$5) + (SELECT count(*) FROM users WHERE id IN ($6,$7,$8)) + (SELECT count(*) FROM messages WHERE channel_id IN ($3,$4)) + (SELECT count(*) FROM channel_user WHERE channel_id IN ($3,$4)) + (SELECT count(*) FROM pg_trigger WHERE tgname=$9) + (SELECT count(*) FROM pg_proc WHERE proname=$10) + (SELECT count(*) FROM pg_class WHERE relname=$11))::text",
            &[run, export, &target, &remapped_target, &team, &admin, &slack_member, &slack_late_author, &trigger, &function, &sequence],
        ).await?;
        anyhow::ensure!(
            rows.first().and_then(|row| row["n"].as_i64()) == Some(0),
            "database fixture rows remain after cleanup: {rows:?}"
        );
        Ok::<(), anyhow::Error>(())
    });
    attempt_cleanup!("verify S3 fixture cleanup", async {
        for id in [run_id.as_deref(), export_id.as_deref()]
            .into_iter()
            .flatten()
        {
            let key = format!("slack-imports/{id}/export.zip");
            assert_s3_object_exists(&state.s3_client, &config.s3.bucket, &key, false).await?;
        }
        Ok::<(), anyhow::Error>(())
    });
    finish_test_result(result, cleanup_errors)
}

async fn list_slack_import_keys(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<std::collections::HashSet<String>> {
    let mut keys = std::collections::HashSet::new();
    let mut token = None;
    let mut seen_tokens = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        anyhow::ensure!(!remaining.is_zero(), "S3 listing exceeded its deadline");
        let page = tokio::time::timeout(
            remaining,
            client
                .list_objects_v2()
                .bucket(bucket)
                .prefix("slack-imports/")
                .set_continuation_token(token.clone())
                .send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("S3 ListObjectsV2 exceeded its deadline"))??;
        keys.extend(
            page.contents()
                .iter()
                .filter_map(|object| object.key())
                .map(str::to_owned),
        );
        if !page.is_truncated().unwrap_or(false) {
            break;
        }
        let next = next_s3_continuation_token(
            token.as_deref(),
            &mut seen_tokens,
            page.next_continuation_token(),
        )?;
        token = Some(next);
    }
    Ok(keys)
}

fn next_s3_continuation_token(
    previous: Option<&str>,
    seen: &mut std::collections::HashSet<String>,
    next: Option<&str>,
) -> anyhow::Result<String> {
    let next =
        next.ok_or_else(|| anyhow::anyhow!("truncated S3 listing lacks continuation token"))?;
    anyhow::ensure!(
        previous != Some(next) && seen.insert(next.to_owned()),
        "S3 ListObjectsV2 continuation token did not advance"
    );
    Ok(next.to_owned())
}

#[test]
fn s3_listing_rejects_repeated_continuation_tokens() {
    let mut seen = std::collections::HashSet::new();
    let first = next_s3_continuation_token(None, &mut seen, Some("page-1")).unwrap();
    assert_eq!(first, "page-1");
    let error = next_s3_continuation_token(Some("page-1"), &mut seen, Some("page-1"))
        .expect_err("a repeated continuation token must stop pagination");
    assert!(error.to_string().contains("token did not advance"));
}

#[tokio::test]
async fn s3_listing_stops_when_the_endpoint_stalls() -> anyhow::Result<()> {
    use aws_credential_types::Credentials;
    use aws_sdk_s3::config::Region;
    use tokio::io::AsyncReadExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = socket.read(&mut request).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    });
    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(Credentials::new("test", "test", None, None, "test"))
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();
    let client = aws_sdk_s3::Client::from_conf(s3_config);
    let result = list_slack_import_keys(
        &client,
        "test-bucket",
        std::time::Duration::from_millis(100),
    )
    .await;
    server.abort();
    anyhow::ensure!(
        result
            .as_ref()
            .err()
            .is_some_and(|error| error.to_string().contains("exceeded its deadline")),
        "a stalled S3 endpoint must fail within the listing deadline: {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn s3_foreign_writer_process_helper() -> anyhow::Result<()> {
    let (Ok(bucket), Ok(key)) = (
        std::env::var("SLACK_IMPORT_FOREIGN_WRITER_BUCKET"),
        std::env::var("SLACK_IMPORT_FOREIGN_WRITER_KEY"),
    ) else {
        return Ok(());
    };
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let config = Config::from_env();
    let client = backend::storage::create_s3_client(&config.s3);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(aws_sdk_s3::primitives::ByteStream::from_static(
                b"other process",
            ))
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("independent process RustFS write timed out"))??;
    Ok(())
}

async fn stop_foreign_writer(child: &mut tokio::process::Child) -> anyhow::Result<()> {
    let status = child.try_wait()?;
    if status.is_none() {
        child.kill().await.map_err(|error| {
            anyhow::anyhow!("stopping timed-out independent RustFS writer: {error}")
        })?;
        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => anyhow::bail!("independent RustFS writer did not exit after kill"),
        }
    } else {
        Ok(())
    }
}

#[tokio::test]
async fn stopping_timed_out_writer_kills_child_and_confirms_exit() -> anyhow::Result<()> {
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()?;
    anyhow::ensure!(
        child.try_wait()?.is_none(),
        "writer fixture exited before timeout handling"
    );
    stop_foreign_writer(&mut child).await?;
    anyhow::ensure!(
        child.try_wait()?.is_some(),
        "writer child still runs after stop confirmation"
    );
    Ok(())
}

async fn run_s3_pagination_fixture(
    object_count: usize,
    inject_body_panic: bool,
) -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.s3.bucket = format!("slack-test-{}", uuid::Uuid::new_v4().simple());
    let client = backend::storage::create_s3_client(&config.s3);
    // CreateBucket may have reached RustFS even if the client times out; track its
    // unique name before issuing the request so cleanup always attempts deletion.
    let bucket_may_exist = true;
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let keys: Vec<_> = (0..object_count)
        .map(|index| format!("slack-imports/{run_id}/{index:04}.zip"))
        .collect();
    let foreign_key = format!(
        "slack-imports/{}/foreign.zip",
        uuid::Uuid::new_v4().simple()
    );
    let mut writer: Option<tokio::process::Child> = None;
    let caught = std::panic::AssertUnwindSafe(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.create_bucket().bucket(&config.s3.bucket).send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("creating disposable RustFS bucket {} timed out", config.s3.bucket))??;
        for key in &keys {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                client.put_object().bucket(&config.s3.bucket).key(key)
                    .body(aws_sdk_s3::primitives::ByteStream::from_static(b"x")).send(),
            ).await.map_err(|_| anyhow::anyhow!("writing disposable pagination fixture {key} timed out"))??;
        }
        let first_page_set = list_slack_import_keys(&client, &config.s3.bucket, std::time::Duration::from_secs(30)).await?;
        anyhow::ensure!(first_page_set.len() == keys.len(), "expected {} fixture objects, got {}", keys.len(), first_page_set.len());
        if inject_body_panic { panic!("injected pagination test body panic"); }
        if object_count <= 1000 { return Ok(()); }

        tokio::time::timeout(std::time::Duration::from_secs(5), client.delete_object().bucket(&config.s3.bucket).key(&keys[0]).send())
            .await.map_err(|_| anyhow::anyhow!("deleting tracked fixture object timed out"))??;
        let executable = std::env::current_exe()?;
        let child = tokio::process::Command::new(executable)
            .args(["--exact", "s3_foreign_writer_process_helper", "--nocapture"])
            .env("SLACK_IMPORT_FOREIGN_WRITER_BUCKET", &config.s3.bucket)
            .env("SLACK_IMPORT_FOREIGN_WRITER_KEY", &foreign_key)
            .env("S3_ENDPOINT", &config.s3.endpoint)
            .env("S3_ACCESS_KEY_ID", &config.s3.access_key_id)
            .env("S3_SECRET_ACCESS_KEY", &config.s3.secret_access_key)
            .env("S3_REGION", &config.s3.region)
            .env("S3_FORCE_PATH_STYLE", config.s3.force_path_style.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        writer = Some(child);
        let writer_wait = tokio::time::timeout(std::time::Duration::from_secs(10), writer.as_mut().expect("writer just spawned").wait()).await;
        let writer_error = match writer_wait {
            Ok(Ok(status)) if status.success() => None,
            Ok(Ok(status)) => Some(format!("independent RustFS writer exited unsuccessfully: {status}")),
            Ok(Err(error)) => Some(format!("waiting for independent RustFS writer failed: {error}")),
            Err(_) => {
                let error = stop_foreign_writer(writer.as_mut().expect("writer just spawned")).await;
                anyhow::bail!("independent RustFS writer process timed out; stop and exit confirmation: {error:?}");
            }
        };
        if let Some(error) = writer_error { anyhow::bail!("{error}"); }
        let after_foreign_write = list_slack_import_keys(&client, &config.s3.bucket, std::time::Duration::from_secs(30)).await?;
        anyhow::ensure!(after_foreign_write.len() == keys.len() && after_foreign_write.contains(&foreign_key), "independent writer's object was not observed: {} objects", after_foreign_write.len());
        anyhow::ensure!(first_page_set != after_foreign_write, "fixture must demonstrate why whole-bucket before/after comparison is unstable");
        Ok(())
    }).catch_unwind().await;
    let mut test_result = match caught {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            Err(anyhow::anyhow!("test body panicked: {message}"))
        }
    };
    let mut cleanup_errors = Vec::new();
    if let Some(child) = writer.as_mut()
        && let Err(error) = stop_foreign_writer(child).await
    {
        cleanup_errors.push(format!("stop independent writer: {error:#}"));
    }
    for key in keys.iter().chain(std::iter::once(&foreign_key)) {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client
                .delete_object()
                .bucket(&config.s3.bucket)
                .key(key)
                .send(),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => cleanup_errors.push(format!("delete {key}: {error}")),
            Err(_) => cleanup_errors.push(format!("delete {key}: timed out")),
        }
    }
    if bucket_may_exist {
        let mut deleted = false;
        for _ in 0..10 {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                client.delete_bucket().bucket(&config.s3.bucket).send(),
            )
            .await
            {
                Ok(Ok(_)) => {
                    deleted = true;
                    break;
                }
                Ok(Err(error)) => cleanup_errors.push(format!(
                    "delete disposable bucket {}: {error}",
                    config.s3.bucket
                )),
                Err(_) => cleanup_errors.push(format!(
                    "delete disposable bucket {}: timed out",
                    config.s3.bucket
                )),
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        if !deleted {
            cleanup_errors.push(format!(
                "disposable bucket may remain or be created after timeout: {}",
                config.s3.bucket
            ));
        } else {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                client.head_bucket().bucket(&config.s3.bucket).send(),
            )
            .await
            {
                Err(_) => cleanup_errors.push(format!(
                    "verify disposable bucket removal {} timed out",
                    config.s3.bucket
                )),
                Ok(Ok(_)) => cleanup_errors.push(format!(
                    "disposable bucket remains after cleanup: {}",
                    config.s3.bucket
                )),
                Ok(Err(error))
                    if error
                        .raw_response()
                        .is_some_and(|response| response.status().as_u16() == 404) => {}
                Ok(Err(error)) => cleanup_errors.push(format!(
                    "verify disposable bucket removal {}: {error}",
                    config.s3.bucket
                )),
            }
        }
    }
    test_result = finish_test_result(test_result, cleanup_errors);
    test_result
}

#[ignore = "requires an explicitly configured disposable RustFS endpoint"]
#[tokio::test]
async fn s3_listing_handles_more_than_one_thousand_keys_and_an_independent_writer()
-> anyhow::Result<()> {
    run_s3_pagination_fixture(1_001, false).await
}

#[ignore = "requires an explicitly configured disposable RustFS endpoint"]
#[tokio::test]
async fn s3_pagination_body_panic_still_cleans_up_disposable_bucket() -> anyhow::Result<()> {
    let error = run_s3_pagination_fixture(1, true)
        .await
        .expect_err("injected body panic must be reported");
    anyhow::ensure!(
        format!("{error:#}").contains("injected pagination test body panic"),
        "unexpected body failure: {error:#}"
    );
    anyhow::ensure!(
        !format!("{error:#}").contains("cleanup failed"),
        "bucket cleanup failed after panic: {error:#}"
    );
    Ok(())
}

async fn assert_s3_object_exists(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    expected: bool,
) -> anyhow::Result<()> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.head_object().bucket(bucket).key(key).send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("S3 HeadObject timed out after 10s for {key}"))?;
    match result {
        Ok(_) if expected => Ok(()),
        Ok(_) => anyhow::bail!("unexpected S3 object remains: {key}"),
        Err(error)
            if !expected
                && error
                    .raw_response()
                    .is_some_and(|r| r.status().as_u16() == 404) =>
        {
            Ok(())
        }
        Err(error) => Err(anyhow::anyhow!("S3 HeadObject failed for {key}: {error}")),
    }
}

async fn delete_s3_object_and_confirm(
    state: &AppState,
    bucket: &str,
    key: &str,
) -> anyhow::Result<()> {
    let delete_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state
            .s3_client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send(),
    )
    .await;
    let delete_error = match delete_result {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(format!("DeleteObject failed: {error}")),
        Err(_) => Some("DeleteObject timed out after 5s".to_string()),
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        assert_s3_object_exists(&state.s3_client, bucket, key, false),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => anyhow::bail!(
            "{}, object removal was not confirmed: {error:#}",
            delete_error.unwrap_or_else(|| "DeleteObject returned success".to_string())
        ),
        Err(_) => anyhow::bail!(
            "{}, object removal verification timed out after 5s",
            delete_error.unwrap_or_else(|| "DeleteObject returned success".to_string())
        ),
    }
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
#[tokio::test]
async fn private_zip_import_stops_after_target_is_public_or_deleted() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    run_private_zip_visibility_race(
        ZipTargetRace::Public,
        true,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    run_private_zip_visibility_race(
        ZipTargetRace::Deleted,
        true,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    run_private_zip_visibility_race(
        ZipTargetRace::Encrypted,
        true,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    run_private_zip_visibility_race(
        ZipTargetRace::Remapped,
        true,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
#[tokio::test]
async fn private_zip_save_guard_rejects_changed_targets_without_a_transient_database_error()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    run_private_zip_visibility_race(
        ZipTargetRace::Public,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    run_private_zip_visibility_race(
        ZipTargetRace::Deleted,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    run_private_zip_visibility_race(
        ZipTargetRace::Encrypted,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    run_private_zip_visibility_race(
        ZipTargetRace::Remapped,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        false,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn start_returns_request_construction_errors() {
    let error = start(&Router::new(), "admin", "bad\nexport", json!([]))
        .await
        .expect_err("an invalid request URI must be returned to the caller");
    assert!(error.to_string().contains("invalid uri"), "{error:#}");
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn private_zip_worker_timeout_aborts_worker_and_releases_test_gate() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    let error = run_private_zip_visibility_race(
        ZipTargetRace::Public,
        false,
        true,
        TestBodyFailure::None,
        CleanupFailure::S3Error,
        false,
    )
    .await
    .expect_err("the deliberately blocked worker must time out");
    anyhow::ensure!(
        format!("{error:#}").contains("injected ZIP worker timeout"),
        "expected the injected worker timeout, got: {error:#}"
    );
    anyhow::ensure!(
        format!("{error:#}").contains("injected S3 cleanup failure")
            && !format!("{error:#}").contains("injected database cleanup failure"),
        "worker timeout and expected S3 cleanup error were not isolated: {error:#}"
    );
    let message = format!("{error:#}");
    let cleanup_errors = message
        .split_once("cleanup also failed: ")
        .map(|(_, errors)| errors)
        .ok_or_else(|| anyhow::anyhow!("cleanup error was not aggregated: {message}"))?;
    let cleanup_errors: Vec<_> = cleanup_errors.split("; ").collect();
    anyhow::ensure!(
        cleanup_errors.len() == 1,
        "unexpected cleanup error also reported: {cleanup_errors:?}"
    );
    anyhow::ensure!(
        cleanup_errors
            .iter()
            .all(|error| error.starts_with("injected S3 cleanup failure")),
        "unexpected cleanup error also reported: {cleanup_errors:?}"
    );
    anyhow::ensure!(
        !message.contains("fixture rows remain") && !message.contains("fixture objects remain"),
        "fixture residue after worker timeout: {message}"
    );
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn private_zip_target_update_lock_timeout_still_aborts_worker_and_releases_gate()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    let error = run_private_zip_visibility_race(
        ZipTargetRace::Public,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::None,
        true,
    )
    .await
    .expect_err("the injected row lock must time out the target update");
    anyhow::ensure!(
        format!("{error:#}").contains("timed out after 5s changing the ZIP destination"),
        "expected the target update timeout, got: {error:#}"
    );
    anyhow::ensure!(
        !format!("{error:#}").contains("cleanup also failed"),
        "target update timeout cleanup failed: {error:#}"
    );
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn private_zip_cleanup_errors_are_reported_and_remaining_cleanup_runs() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    let error = run_private_zip_visibility_race(
        ZipTargetRace::Public,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::DatabaseError,
        false,
    )
    .await
    .expect_err("the deliberately failed cleanup operations must be reported");
    let message = format!("{error:#}");
    anyhow::ensure!(
        !message.contains("injected S3 cleanup failure"),
        "{message}"
    );
    anyhow::ensure!(
        message.contains("injected database cleanup failure"),
        "{message}"
    );
    anyhow::ensure!(
        message.contains("test succeeded, but cleanup failed"),
        "{message}"
    );
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn private_zip_reports_real_job_delete_failure_and_recovers_rows() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    let error = run_private_zip_visibility_race(
        ZipTargetRace::Public,
        false,
        false,
        TestBodyFailure::None,
        CleanupFailure::JobDeleteError,
        false,
    )
    .await
    .expect_err("the injected real slack_import_jobs DELETE failure must be reported");
    let message = format!("{error:#}");
    anyhow::ensure!(
        message.contains("delete export job"),
        "DELETE error not reported: {message}"
    );
    anyhow::ensure!(
        message.contains("injected job DELETE failure"),
        "actual SQL DELETE was not rejected by injection: {message}"
    );
    anyhow::ensure!(
        message.contains("test succeeded, but cleanup failed"),
        "cleanup failure was not surfaced: {message}"
    );
    anyhow::ensure!(
        !message.contains("database fixture rows remain"),
        "rows were not recovered after proving they remained: {message}"
    );
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn private_zip_body_panics_and_empty_results_still_clean_up() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    for (failure, expected) in [
        (TestBodyFailure::Panic, "injected ZIP test body panic"),
        (
            TestBodyFailure::EmptyResult,
            "injected empty ZIP test result",
        ),
    ] {
        let error = run_private_zip_visibility_race(
            ZipTargetRace::Public,
            false,
            false,
            failure,
            CleanupFailure::None,
            false,
        )
        .await
        .expect_err("the injected test body failure must be returned");
        anyhow::ensure!(format!("{error:#}").contains(expected), "{error:#}");
        anyhow::ensure!(
            !format!("{error:#}").contains("cleanup also failed"),
            "fixture cleanup failed after body failure: {error:#}"
        );
    }
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn private_zip_cleanup_errors_do_not_hide_other_cleanup_and_leave_no_fixtures()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    for (failure, expected) in [
        (
            CleanupFailure::DatabaseError,
            "injected database cleanup failure",
        ),
        (CleanupFailure::S3Error, "injected S3 cleanup failure"),
        (
            CleanupFailure::DatabaseTimeout,
            "injected database cleanup timeout: timed out after 5s",
        ),
        (
            CleanupFailure::S3Timeout,
            "injected S3 cleanup timeout: timed out after 5s",
        ),
    ] {
        let error = run_private_zip_visibility_race(
            ZipTargetRace::Public,
            false,
            false,
            TestBodyFailure::None,
            failure,
            false,
        )
        .await
        .expect_err("the injected cleanup failure must be reported");
        let message = format!("{error:#}");
        anyhow::ensure!(
            message.contains(expected),
            "expected {expected:?}: {message}"
        );
        if matches!(failure, CleanupFailure::S3Error | CleanupFailure::S3Timeout) {
            anyhow::ensure!(
                message.contains("recovery resources: job_id=")
                    && message.contains("bucket=")
                    && message.contains("key=slack-imports/"),
                "an unconfirmed S3 deletion must keep its job and report the recovery location: {message}"
            );
        }
        anyhow::ensure!(
            message.contains("test succeeded, but cleanup failed"),
            "{message}"
        );
        let expected_label = match failure {
            CleanupFailure::DatabaseError | CleanupFailure::DatabaseTimeout => "database cleanup",
            CleanupFailure::S3Error | CleanupFailure::S3Timeout => "S3 cleanup",
            CleanupFailure::JobDeleteError => "job DELETE cleanup",
            CleanupFailure::None => unreachable!(),
        };
        anyhow::ensure!(
            !message.contains(if expected_label == "database cleanup" {
                "injected S3 cleanup"
            } else {
                "injected database cleanup"
            }),
            "unexpected second cleanup failure was masked or injected: {message}"
        );
        let reported_cleanup = message
            .split_once("cleanup failed: ")
            .map(|(_, errors)| errors)
            .ok_or_else(|| anyhow::anyhow!("cleanup errors were not aggregated: {message}"))?;
        let reported_errors: Vec<_> = reported_cleanup.split("; ").collect();
        let expected_count = 1;
        anyhow::ensure!(
            reported_errors.len() == expected_count,
            "unexpected cleanup error also reported: {reported_errors:?}"
        );
        anyhow::ensure!(
            reported_errors
                .iter()
                .all(|error| error.starts_with(expected)),
            "unexpected cleanup error also reported: {reported_errors:?}"
        );
        anyhow::ensure!(
            !message.contains("database fixture rows remain")
                && !message.contains("S3 fixture objects remain"),
            "injected cleanup error left fixture residue: {message}"
        );
    }
    Ok(())
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn failed_upload_cleanup_aggregates_db_and_s3_errors_and_timeouts() -> anyhow::Result<()> {
    run_failed_upload_cleanup_fixture(false).await
}

#[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL and RustFS"]
#[tokio::test]
async fn failed_upload_cleanup_catches_body_panic_and_cleans_disposable_bucket()
-> anyhow::Result<()> {
    run_failed_upload_cleanup_fixture(true).await
}

async fn run_failed_upload_cleanup_fixture(inject_body_panic: bool) -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.database_url = std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
    config.s3.bucket = format!("slack-test-{}", uuid::Uuid::new_v4().simple());
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config.clone());
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_upload_cleanup_admin").await?;
    // Mark the bucket as potentially present before sending CreateBucket: a timeout only
    // cancels the client wait and does not prove that RustFS stopped processing the request.
    let mut bucket_may_exist = false;

    let caught = std::panic::AssertUnwindSafe(async {
        bucket_may_exist = true;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            state.s3_client.create_bucket().bucket(&config.s3.bucket).send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("creating disposable upload cleanup bucket {} timed out", config.s3.bucket))??;
        cleanup_statement(
            &state,
            "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin')",
            &[&admin],
        )
        .await?;
        let before_jobs = sql(
            &state,
            "SELECT json_build_object('n',count(*))::text FROM slack_import_jobs WHERE created_by=$1",
            &[&admin],
        )
        .await?;
        anyhow::ensure!(before_jobs.first().and_then(|row| row["n"].as_i64()) == Some(0), "unexpected pre-existing upload job: {before_jobs:?}");
        for (failure, expected, unexpected, remaining_jobs, remaining_objects) in [
            (UploadCleanupFailure::DatabaseError, "injected database cleanup failure", "injected S3 cleanup", 1, 0),
            (UploadCleanupFailure::S3Error, "injected S3 cleanup failure", "injected database cleanup", 1, 1),
            (UploadCleanupFailure::DatabaseTimeout, "injected database cleanup timeout: timed out after 5s", "injected S3 cleanup", 1, 0),
            (UploadCleanupFailure::S3Timeout, "injected S3 cleanup timeout: timed out after 5s", "injected database cleanup", 1, 1),
        ] {
            let error = upload_with_cleanup_failure(
                &state,
                &app,
                &admin,
                &build_zip(&[]),
                failure,
            )
            .await
            .expect_err("injected upload failure must be returned");
            let message = format!("{error:#}");
            anyhow::ensure!(message.contains("injected upload failure after ZIP object creation"), "{message}");
            anyhow::ensure!(message.contains(expected), "expected {expected:?}: {message}");
            anyhow::ensure!(message.contains(&format!("bucket={}, key=slack-imports/", config.s3.bucket)), "recovery location missing: {message}");
            anyhow::ensure!(!message.contains(unexpected), "unexpected second cleanup failure: {message}");
            let reported_cleanup = message
                .split_once("cleanup failed: ")
                .map(|(_, errors)| errors)
                .ok_or_else(|| anyhow::anyhow!("cleanup errors were not aggregated: {message}"))?;
            anyhow::ensure!(reported_cleanup.split("; ").count() == 1, "unexpected cleanup error also reported: {reported_cleanup}");
            let job_id = message.split_once("recovery resources: job_id=").and_then(|(_, tail)| tail.split_once(',').map(|(id, _)| id)).ok_or_else(|| anyhow::anyhow!("job recovery id missing: {message}"))?;
            let rows = sql(&state, "SELECT json_build_object('n',count(*))::text FROM slack_import_jobs WHERE id=$1", &[job_id]).await?;
            anyhow::ensure!(rows.first().and_then(|row| row["n"].as_i64()) == Some(remaining_jobs), "unexpected retained job state: {rows:?}");
            let object_exists = assert_s3_object_exists(&state.s3_client, &config.s3.bucket, &format!("slack-imports/{job_id}/export.zip"), remaining_objects == 1).await.is_ok();
            anyhow::ensure!(object_exists, "unexpected retained ZIP object state for job {job_id}");
            // The test owns these rows and objects; recover even intentionally failed cleanup here.
            state.s3_client.delete_object().bucket(&config.s3.bucket).key(format!("slack-imports/{job_id}/export.zip")).send().await?;
            cleanup_statement(&state, "DELETE FROM slack_import_jobs WHERE id=$1", &[job_id]).await?;
        }
        let stranded_zip = build_zip(&[]);
        let (status, created) = request(
            &app,
            "POST",
            "/api/admin/slack-imports",
            &admin,
            Some(json!({"file_name":"stranded.zip","file_size":stranded_zip.len()})),
        )
        .await?;
        anyhow::ensure!(status == StatusCode::OK, "stranded fixture job creation failed: {created}");
        let stranded_id = created["import"]["id"].as_str().ok_or_else(|| anyhow::anyhow!("missing stranded job id: {created}"))?;
        let response = reqwest::Client::new()
            .put(created["upload_url"].as_str().ok_or_else(|| anyhow::anyhow!("missing stranded upload URL: {created}"))?)
            .header("content-type", "application/zip")
            .body(stranded_zip)
            .send()
            .await?;
        anyhow::ensure!(response.status().is_success(), "stranded ZIP upload failed: {}", response.text().await.unwrap_or_default());
        if inject_body_panic {
            panic!("injected upload cleanup test body panic after durable ZIP: job_id={stranded_id}");
        }
        anyhow::bail!("injected assertion after durable admin job and RustFS object: job_id={stranded_id}")
    }).catch_unwind().await;
    let test_result = match caught {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            Err(anyhow::anyhow!("test body panicked: {message}"))
        }
    };

    let mut cleanup_errors = Vec::new();
    let jobs = match sql(
        &state,
        "SELECT json_build_object('id',id,'storage_key',storage_key)::text FROM slack_import_jobs WHERE created_by=$1 ORDER BY (source_id IS NULL),id",
        &[&admin],
    )
    .await
    {
        Ok(jobs) => jobs,
        Err(error) => {
            cleanup_errors.push(format!("list admin-owned upload jobs: {error:#}"));
            Vec::new()
        }
    };
    for job in jobs {
        let Some(job_id) = job["id"].as_str() else {
            cleanup_errors.push(format!("admin-owned upload job has no id: {job:?}"));
            continue;
        };
        let Some(key) = job["storage_key"].as_str() else {
            cleanup_errors.push(format!(
                "admin-owned upload job {job_id} has no storage key"
            ));
            continue;
        };
        match delete_s3_object_and_confirm(&state, &config.s3.bucket, key).await {
            Ok(()) => {}
            Err(error) => {
                cleanup_errors.push(format!(
                    "retain admin-owned job after unconfirmed ZIP deletion: {error:#}, recovery resources: job_id={job_id}, bucket={}, key={key}",
                    config.s3.bucket
                ));
                continue;
            }
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            cleanup_statement(&state, "DELETE FROM slack_import_jobs WHERE id=$1", &[job_id]),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => cleanup_errors.push(format!(
                "delete admin-owned job {job_id} after confirmed object removal: {error:#}, bucket={}, key={key}",
                config.s3.bucket
            )),
            Err(_) => cleanup_errors.push(format!(
                "delete admin-owned job {job_id} after confirmed object removal: timed out after 5s, bucket={}, key={key}",
                config.s3.bucket
            )),
        }
    }
    match list_slack_import_keys(
        &state.s3_client,
        &config.s3.bucket,
        std::time::Duration::from_secs(30),
    )
    .await
    {
        Ok(keys) => {
            for key in keys {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    delete_s3_object_and_confirm(&state, &config.s3.bucket, &key),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        cleanup_errors.push(format!("delete residual S3 key {key}: {error}"))
                    }
                    Err(_) => {
                        cleanup_errors.push(format!("delete residual S3 key {key}: timed out"))
                    }
                }
            }
        }
        Err(error) => cleanup_errors.push(format!("list disposable bucket for cleanup: {error:#}")),
    }
    for (label, statement) in [
        (
            "delete admin role",
            "DELETE FROM user_role WHERE user_id=$1",
        ),
        ("delete admin user", "DELETE FROM users WHERE id=$1"),
    ] {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            cleanup_statement(&state, statement, &[&admin]),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => cleanup_errors.push(format!("{label}: {error:#}")),
            Err(_) => cleanup_errors.push(format!("{label}: timed out after 5s")),
        }
    }
    if bucket_may_exist {
        // Retry because a timed-out CreateBucket request may still be completing server-side.
        let mut deleted = false;
        for _ in 0..10 {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                state
                    .s3_client
                    .delete_bucket()
                    .bucket(&config.s3.bucket)
                    .send(),
            )
            .await
            {
                Ok(Ok(_)) => {
                    deleted = true;
                    break;
                }
                Ok(Err(error)) => cleanup_errors.push(format!(
                    "delete disposable bucket {}: {error}",
                    config.s3.bucket
                )),
                Err(_) => cleanup_errors.push(format!(
                    "delete disposable bucket {}: timed out",
                    config.s3.bucket
                )),
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        if !deleted {
            cleanup_errors.push(format!(
                "disposable bucket may remain or be created after timeout: {}",
                config.s3.bucket
            ));
        }
    }
    let result = finish_test_result(test_result, cleanup_errors);
    let error = result.expect_err("the injected assertion must be returned after cleanup");
    let message = format!("{error:#}");
    let expected_body = if inject_body_panic {
        "test body panicked: injected upload cleanup test body panic after durable ZIP"
    } else {
        "injected assertion after durable admin job and RustFS object"
    };
    anyhow::ensure!(message.contains(expected_body), "{message}");
    anyhow::ensure!(
        !message.contains("cleanup also failed"),
        "fixture recovery failed: {message}"
    );
    let remaining_jobs = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM slack_import_jobs WHERE created_by=$1",
        &[&admin],
    )
    .await?;
    anyhow::ensure!(
        remaining_jobs.first().and_then(|row| row["n"].as_i64()) == Some(0),
        "admin-owned upload job remains: {remaining_jobs:?}"
    );
    let remaining_user = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM users WHERE id=$1",
        &[&admin],
    )
    .await?;
    anyhow::ensure!(
        remaining_user.first().and_then(|row| row["n"].as_i64()) == Some(0),
        "admin user remains after job cleanup: {remaining_user:?}"
    );
    let bucket = state
        .s3_client
        .head_bucket()
        .bucket(&config.s3.bucket)
        .send()
        .await;
    anyhow::ensure!(
        bucket
            .as_ref()
            .err()
            .and_then(|error| error.raw_response())
            .is_some_and(|response| response.status().as_u16() == 404),
        "disposable upload bucket was not confirmed absent after fixture cleanup: {} ({bucket:?})",
        config.s3.bucket
    );
    Ok(())
}

#[tokio::test]
async fn test_slack_import_rejects_invalid_archives_and_can_be_cancelled() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin2").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    // 上限を超えるサイズは受け付けない
    let (status, _) = request(
        &app,
        "POST",
        "/api/admin/slack-imports",
        &admin,
        Some(json!({"file_name": "export.zip", "file_size": config.slack.import_max_zip_size + 1})),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // アップロード前に解析を依頼しても通らない
    let (_, created) = request(
        &app,
        "POST",
        "/api/admin/slack-imports",
        &admin,
        Some(json!({"file_name": "export.zip", "file_size": 4})),
    )
    .await?;
    let id = created["import"]["id"].as_str().unwrap().to_string();
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{id}/uploaded"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, cancelled) = request(
        &app,
        "DELETE",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["status"], "cancelled");

    // channels.json がない ZIP はすぐ失敗にする
    let zip = build_zip(&[("users.json".into(), json!([]))]);
    let (_, created) = request(
        &app,
        "POST",
        "/api/admin/slack-imports",
        &admin,
        Some(json!({"file_name": "broken.zip", "file_size": zip.len()})),
    )
    .await?;
    reqwest::Client::new()
        .put(created["upload_url"].as_str().unwrap())
        .header("content-type", "application/zip")
        .body(zip)
        .send()
        .await?
        .error_for_status()?;
    let id = created["import"]["id"].as_str().unwrap().to_string();
    request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{id}/uploaded"),
        &admin,
        None,
    )
    .await?;
    let mut job = Value::Null;
    for _ in 0..100 {
        backend::slack::worker::run_once(&state).await?;
        job = request(
            &app,
            "GET",
            &format!("/api/admin/slack-imports/{id}"),
            &admin,
            None,
        )
        .await?
        .1;
        if job["status"] == "failed" {
            break;
        }
    }
    assert_eq!(job["status"], "failed");
    assert!(job["error"].as_str().unwrap().contains("channels.json"));
    Ok(())
}

#[tokio::test]
async fn test_slack_reimport_relinks_replies_and_serializes_targets() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin3").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let users = json!([{"id":"U1","team_id":team,"name":"alice"}]);
    let channels = json!([{"id":"C1","name":"general","members":["U1"]}]);
    let parent = json!({"ts":"1704070000.000100","user":"U1","text":"parent","thread_ts":"1704070000.000100"});
    let reply = json!({"ts":"1704160000.000200","user":"U1","text":"reply","thread_ts":"1704070000.000100"});
    // 1 回目は返信だけ (親は期間外)
    let replies_only = build_zip(&[
        ("users.json".into(), users.clone()),
        ("channels.json".into(), channels.clone()),
        ("general/2024-01-02.json".into(), json!([reply.clone()])),
    ]);
    let both = build_zip(&[
        ("users.json".into(), users),
        ("channels.json".into(), channels),
        (
            "general/2024-01-01.json".into(),
            json!([
                parent,
                // 本文がカード (attachments) にしかない bot 投稿
                {"ts":"1704070100.000300","bot_id":"B1","subtype":"bot_message","username":"deploybot","text":"",
                 "attachments":[
                    {"title":"Deploy","text":"production is ready","color":"good","fields":[{"title":"env","value":"prod"}]},
                    {"from_url":"https://example.com","title":"unfurl","text":"link preview"}
                 ]},
                // 本文とカードの両方がある bot 投稿
                {"ts":"1704070150.000350","bot_id":"B1","subtype":"bot_message","username":"alertbot",
                 "text":"Outage in <#C1|general>, see <https://status.example.com|status>",
                 "attachments":[{"title":"Status","text":"degraded","color":"danger"}]},
                // 取り込めないファイル (外部サービスの共有) だけの投稿は、ファイル名を本文に残す
                {"ts":"1704070160.000360","user":"U1","text":"",
                 "files":[{"id":"FEXT","name":"Roadmap.gdoc","mode":"external","url_private_download":"https://docs.example.com/x"}]},
                // 名前のない削除済みファイルだけの投稿は、空の投稿にせず取り込まない
                {"ts":"1704070170.000370","user":"U1","text":"","files":[{"id":"FDEL","mode":"tombstone"}]},
                // 取得をスキップする (上限を超える) ファイルだけの投稿も、ファイル名を本文に残す
                {"ts":"1704070180.000380","user":"U1","text":"",
                 "files":[{"id":"FBIG","name":"huge.bin","size":999999999999i64,"url_private_download":"https://files.slack.com/files-pri/T1-FBIG/huge.bin"}]}
            ]),
        ),
        ("general/2024-01-02.json".into(), json!([reply])),
    ]);

    // 同じ Slack チャンネルを別々の取り込み先へ入れるジョブを、どちらも開始してしまう
    let export = upload(&state, &app, &admin, &replies_only).await?;
    // 同じ ZIP から何度取り込んでも、保存中の ZIP は一覧から押し出されない
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement(
            "INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id,source_id) \
             SELECT gen_random_uuid()::text,created_by,'completed',file_name,file_size,storage_key,team_id,id \
             FROM slack_import_jobs, generate_series(1,60) WHERE id=$1",
        )
        .bind(export["id"].as_str().unwrap())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }
    let (status, listed) = request(&app, "GET", "/api/admin/slack-imports", &admin, None).await?;
    assert_eq!(status, StatusCode::OK);
    let listed = listed["imports"].as_array().unwrap();
    assert!(
        listed.iter().any(|job| job["id"] == export["id"]),
        "the stored export stays listed"
    );
    // 取得し直せる添付が残る完了ジョブは件数制限の対象外なので、それ以外の完了ジョブだけを数える
    assert!(
        listed
            .iter()
            .filter(|job| job["status"] == "completed"
                && job["pending_files"] == 0
                && job["failed_files"] == 0
                && job["skipped_files"] == 0)
            .count()
            <= 50
    );
    let export_id = export["id"].as_str().unwrap().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &app,
        "POST",
        "/api/channels",
        &admin,
        Some(json!({"id": other, "channel_name": format!("other_{suffix}"), "is_private": false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let mut runs = Vec::new();
    for mapping in [
        json!({"slack_channel_id":"C1","action":"create","name":format!("first-{suffix}")}),
        json!({"slack_channel_id":"C1","action":"existing","channel_id":other}),
    ] {
        let (status, body) = start(&app, &admin, &export_id, json!([mapping])).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        runs.push(body["id"].as_str().unwrap().to_string());
    }
    let (first_id, second_id) = (runs[0].clone(), runs[1].clone());
    // 先に処理されたほうだけが成功し、もう一方は取り込み先が食い違うので失敗する
    drive(&state, &app, &admin, &first_id, "completed").await?;
    let mut failed = Value::Null;
    for _ in 0..100 {
        backend::slack::worker::run_once(&state).await?;
        failed = request(
            &app,
            "GET",
            &format!("/api/admin/slack-imports/{second_id}"),
            &admin,
            None,
        )
        .await?
        .1;
        if failed["status"] == "failed" {
            break;
        }
    }
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("already imported")
    );

    let general = backend::slack::channel_id(&team, "C1");
    let reply_id = backend::slack::message_id(&team, "C1", "1704160000.000200");
    let parent_of = |rows: Vec<Value>| rows[0]["parent_id"].clone();
    let rows = sql(
        &state,
        "SELECT json_build_object('parent_id',parent_id)::text FROM messages WHERE id=$1",
        &[&reply_id],
    )
    .await?;
    assert_eq!(
        parent_of(rows),
        Value::Null,
        "the reply is a root until its parent arrives"
    );

    // 親を含む ZIP を取り込み直すと、返信がスレッドに戻る
    let again = upload(&state, &app, &admin, &both).await?;
    let (status, body) = start(
        &app,
        &admin,
        again["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":general}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(
        &state,
        &app,
        &admin,
        body["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let rows = sql(
        &state,
        "SELECT json_build_object('parent_id',parent_id)::text FROM messages WHERE id=$1",
        &[&reply_id],
    )
    .await?;
    assert_eq!(
        parent_of(rows),
        json!(backend::slack::message_id(&team, "C1", "1704070000.000100"))
    );

    // カードだけの bot 投稿も取り込まれ、カードと本文が保存される (リンクの展開は除く)
    let card = sql(
        &state,
        "SELECT json_build_object('content',content,'card',card)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(
            &team,
            "C1",
            "1704070100.000300",
        )],
    )
    .await?;
    assert_eq!(card.len(), 1, "the card-only bot message must be imported");
    assert_eq!(card[0]["content"], "Deploy\nproduction is ready\nenv: prod");
    let stored: Value = serde_json::from_str(card[0]["card"].as_str().unwrap())?;
    assert_eq!(stored["title"], "Deploy");
    assert!(!card[0]["card"].as_str().unwrap().contains("link preview"));

    // 本文とカードの両方がある投稿は、本文もカードの先頭に入り、チャットで両方が読める
    let both = sql(
        &state,
        "SELECT json_build_object('content',content,'card',card)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(
            &team,
            "C1",
            "1704070150.000350",
        )],
    )
    .await?;
    assert_eq!(both.len(), 1);
    let content = both[0]["content"].as_str().unwrap().to_string();
    // チャンネルへのリンクは取り込み先の名前に、リンクは MFM に直る
    assert!(
        content.starts_with("Outage in #first-")
            && content.ends_with("[status](https://status.example.com)"),
        "{content}"
    );
    let stored: Value = serde_json::from_str(both[0]["card"].as_str().unwrap())?;
    let cards = stored["cards"]
        .as_array()
        .expect("the text and the attachment");
    assert_eq!(cards.len(), 2);
    // カードの説明は MFM として描画しないので、リンクは「ラベル (URL)」で残す
    assert_eq!(cards[0]["title"], "Slack");
    let description = cards[0]["description"].as_str().unwrap();
    assert!(
        description.starts_with("Outage in #first-")
            && description.ends_with("see status (https://status.example.com)"),
        "{description}"
    );
    assert_eq!(cards[1]["title"], "Status");
    let files_only = sql(
        &state,
        "SELECT json_build_object('id',id,'content',content)::text FROM messages WHERE id IN ($1,$2)",
        &[
            &backend::slack::message_id(&team, "C1", "1704070160.000360"),
            &backend::slack::message_id(&team, "C1", "1704070170.000370"),
        ],
    )
    .await?;
    assert_eq!(files_only.len(), 1, "{files_only:?}");
    assert_eq!(files_only[0]["content"], "Roadmap.gdoc");
    let skipped = sql(
        &state,
        "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(
            &team,
            "C1",
            "1704070180.000380",
        )],
    )
    .await?;
    assert_eq!(skipped[0]["content"], "huge.bin");
    Ok(())
}

#[tokio::test]
async fn test_slack_import_guards_private_channels_and_restores_threads() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin4").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    // team_id がどこにもない ZIP は、どのワークスペースか決められないので失敗にする
    let anonymous = build_zip(&[
        ("users.json".into(), json!([{"id":"U1","name":"alice"}])),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":"general"}]),
        ),
    ]);
    let (_, created) = request(
        &app,
        "POST",
        "/api/admin/slack-imports",
        &admin,
        Some(json!({"file_name": "anonymous.zip", "file_size": anonymous.len()})),
    )
    .await?;
    reqwest::Client::new()
        .put(created["upload_url"].as_str().unwrap())
        .header("content-type", "application/zip")
        .body(anonymous)
        .send()
        .await?
        .error_for_status()?;
    let anonymous_id = created["import"]["id"].as_str().unwrap().to_string();
    request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{anonymous_id}/uploaded"),
        &admin,
        None,
    )
    .await?;
    let mut job = Value::Null;
    for _ in 0..100 {
        backend::slack::worker::run_once(&state).await?;
        job = request(
            &app,
            "GET",
            &format!("/api/admin/slack-imports/{anonymous_id}"),
            &admin,
            None,
        )
        .await?
        .1;
        if job["status"] == "failed" {
            break;
        }
    }
    assert_eq!(job["status"], "failed", "{job}");

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let users = json!([
        {"id":"U1","team_id":team,"name":"alice"},
        {"id":"U2","team_id":team,"name":"bob"},
        {"id":"U9","team_id":team,"name":"carol"}
    ]);
    let channels = json!([
        {"id":"C1","name":"general","members":["U1"]},
        {"id":"C2","name":"random","members":["U1"]}
    ]);
    let groups = json!([{"id":"G1","name":"secret","members":["U1"]}]);
    let parent_ts = "1704070000.000100";
    let reply_ts = "1704160000.000200";
    let reply = json!({"ts":reply_ts,"user":"U2","text":"reply","thread_ts":parent_ts});
    let first = build_zip(&[
        ("users.json".into(), users.clone()),
        ("channels.json".into(), channels.clone()),
        ("groups.json".into(), groups.clone()),
        (
            "general/2024-01-02.json".into(),
            json!([
                reply,
                // メンションされただけの人もプレースホルダとして解決される
                {"ts":"1704160100.000300","user":"U1","text":"hi <@U9>"},
                // http のリンクしかない bot のカードも、リンクを外して取り込む
                {"ts":"1704160200.000400","bot_id":"B1","subtype":"bot_message","username":"ci","text":"",
                 "attachments":[{"title":"Build","title_link":"http://ci.example.com/1","fallback":"Build passed"}]}
            ]),
        ),
        (
            "random/2024-01-02.json".into(),
            json!([{"ts":"1704160000.000500","user":"U1","text":"x"}]),
        ),
        (
            "secret/2024-01-02.json".into(),
            json!([{"ts":"1704160000.000600","user":"U1","text":"s"}]),
        ),
    ]);
    let analyzed = upload(&state, &app, &admin, &first).await?;
    let id = analyzed["id"].as_str().unwrap().to_string();

    // 非公開チャンネルを公開チャンネルとして作る・公開チャンネルへ入れる指定は拒否する
    let public = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &app,
        "POST",
        "/api/channels",
        &admin,
        Some(
            json!({"id": public, "channel_name": format!("public_{suffix}"), "is_private": false}),
        ),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    for mapping in [
        json!({"slack_channel_id":"G1","action":"create","name":format!("secret-{suffix}"),"is_private":false}),
        json!({"slack_channel_id":"G1","action":"existing","channel_id":public}),
    ] {
        let (status, body) = request(
            &app,
            "POST",
            &format!("/api/admin/slack-imports/{id}/start"),
            &admin,
            Some(json!({ "mappings": [mapping] })),
        )
        .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    // 件数は選んだチャンネルの分だけ数える
    let (status, started) = start(
        &app,
        &admin,
        &id,
        json!([
            {"slack_channel_id":"C1","action":"create","name":format!("general-{suffix}")},
            {"slack_channel_id":"C2","action":"skip"},
            {"slack_channel_id":"G1","action":"create","name":format!("secret-{suffix}"),"is_private":true}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["total_messages"], 4);
    let run = started["id"].as_str().unwrap().to_string();
    // 取り込み中はエクスポートを消せない (ZIP を読んでいるため)
    let (status, _) = request(
        &app,
        "DELETE",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    let done = drive(&state, &app, &admin, &run, "completed").await?;
    assert_eq!(done["processed_messages"], 4);

    let message = |ts: &str| backend::slack::message_id(&team, "C1", ts);
    let rows = sql(
        &state,
        "SELECT json_build_object('content',m.content,'card',m.card,'user_name',u.user_name)::text \
         FROM messages m JOIN users u ON u.id=m.user_id WHERE m.id=$1",
        &[&message("1704160200.000400")],
    )
    .await?;
    assert_eq!(rows.len(), 1, "the http-only card must be imported");
    assert!(
        !rows[0]["card"]
            .as_str()
            .unwrap_or_default()
            .contains("http://")
    );
    assert!(rows[0]["content"].as_str().unwrap().contains("Build"));
    let rows = sql(
        &state,
        "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
        &[&message("1704160100.000300")],
    )
    .await?;
    let mention = rows[0]["content"].as_str().unwrap();
    assert!(mention.starts_with("hi @slack-carol"), "{mention}");

    // 親だけの ZIP を後から取り込んでも、replies に挙がっている返信がスレッドに戻る
    let parent_only = build_zip(&[
        ("users.json".into(), users),
        ("channels.json".into(), channels),
        (
            "general/2024-01-01.json".into(),
            json!([{"ts":parent_ts,"user":"U1","text":"parent","thread_ts":parent_ts,
                    "replies":[{"user":"U2","ts":reply_ts}]}]),
        ),
    ]);
    let again = upload(&state, &app, &admin, &parent_only).await?;
    let general = backend::slack::channel_id(&team, "C1");
    let (status, body) = start(
        &app,
        &admin,
        again["id"].as_str().unwrap(),
        json!([
            {"slack_channel_id":"C1","action":"existing","channel_id":general},
            {"slack_channel_id":"C2","action":"skip"}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(
        &state,
        &app,
        &admin,
        body["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let rows = sql(
        &state,
        "SELECT json_build_object('parent_id',parent_id)::text FROM messages WHERE id=$1",
        &[&message(reply_ts)],
    )
    .await?;
    assert_eq!(rows[0]["parent_id"], json!(message(parent_ts)));

    // 公開で取り込んだ後に Slack 側で非公開になったチャンネルは、create で指定しても公開先へ入れない
    let users = json!([{"id":"U1","team_id":team,"name":"alice"}]);
    let turned_private = build_zip(&[
        ("users.json".into(), users),
        ("channels.json".into(), json!([])),
        (
            "groups.json".into(),
            json!([
                {"id":"C1","name":"general","members":["U1"]},
                {"id":"G1","name":"secret","members":["U1"]}
            ]),
        ),
        (
            "general/2024-01-03.json".into(),
            json!([{"ts":"1704250000.000700","user":"U1","text":"now private"}]),
        ),
        (
            "secret/2024-01-03.json".into(),
            json!([{"ts":"1704250000.000800","user":"U1","text":"still private"}]),
        ),
    ]);
    let analyzed = upload(&state, &app, &admin, &turned_private).await?;
    let export = analyzed["id"].as_str().unwrap().to_string();
    let (status, body) = start(
        &app,
        &admin,
        &export,
        json!([
            {"slack_channel_id":"C1","action":"create","name":format!("general-{suffix}"),"is_private":true},
            {"slack_channel_id":"G1","action":"skip"}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let private_message = backend::slack::message_id(&team, "C1", "1704250000.000700");
    let rows = sql(
        &state,
        "SELECT json_build_object('id',id)::text FROM messages WHERE id=$1",
        &[&private_message],
    )
    .await?;
    assert!(rows.is_empty());

    // 開始後に取り込み先が公開に変わった場合も、ワーカーが非公開の履歴を入れずに失敗させる
    let secret = backend::slack::channel_id(&team, "G1");
    let (status, body) = start(
        &app,
        &admin,
        &export,
        json!([
            {"slack_channel_id":"C1","action":"skip"},
            {"slack_channel_id":"G1","action":"create","name":format!("secret-{suffix}"),"is_private":true}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let run = body["id"].as_str().unwrap().to_string();
    sql(
        &state,
        "UPDATE channels SET is_private=false WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&secret],
    )
    .await?;
    let mut job = Value::Null;
    for _ in 0..100 {
        backend::slack::worker::run_once(&state).await?;
        job = request(
            &app,
            "GET",
            &format!("/api/admin/slack-imports/{run}"),
            &admin,
            None,
        )
        .await?
        .1;
        if job["status"] == "failed" {
            break;
        }
    }
    assert_eq!(job["status"], "failed", "{job}");
    assert!(
        job["error"]
            .as_str()
            .unwrap_or_default()
            .contains("public channel"),
        "{job}"
    );
    let rows = sql(
        &state,
        "SELECT json_build_object('id',id)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(
            &team,
            "G1",
            "1704250000.000800",
        )],
    )
    .await?;
    assert!(rows.is_empty());
    // 非公開の取り込み先の今の名前は、公開チャンネルの本文に書かない
    sql(
        &state,
        "UPDATE channels SET is_private=true,channel_name=$2 WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&secret, &format!("hidden-{suffix}")],
    )
    .await?;
    // 後から別のチャンネルだけを取り込んでも、以前取り込んだチャンネルへの参照は取り込み先の名前になる
    let later = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([
                {"id":"C1","name":"general","members":["U1"]},
                {"id":"C2","name":"random","members":["U1"]}
            ]),
        ),
        (
            "random/2024-01-04.json".into(),
            json!([{"ts":"1704340000.000900","user":"U1","text":"see <#C1|general> and <#G1|secret>"}]),
        ),
    ]);
    let analyzed = upload(&state, &app, &admin, &later).await?;
    let (status, body) = start(
        &app,
        &admin,
        analyzed["id"].as_str().unwrap(),
        json!([
            {"slack_channel_id":"C1","action":"skip"},
            {"slack_channel_id":"C2","action":"create","name":format!("random-{suffix}")}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(
        &state,
        &app,
        &admin,
        body["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let rows = sql(
        &state,
        "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(
            &team,
            "C2",
            "1704340000.000900",
        )],
    )
    .await?;
    assert_eq!(
        rows[0]["content"],
        format!("see #general-{suffix} and #secret")
    );

    // 以前非公開で作ったチャンネルを「新しいチャンネル (公開)」で選び直しても、実際は非公開なので名前を本文に書かない
    let quiet = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([
                {"id":"C4","name":"quiet","members":["U1"]},
                {"id":"C5","name":"loud","members":["U1"]}
            ]),
        ),
        (
            "quiet/2024-01-05.json".into(),
            json!([{"ts":"1704430000.000100","user":"U1","text":"hush"}]),
        ),
        (
            "loud/2024-01-05.json".into(),
            json!([{"ts":"1704430000.000200","user":"U1","text":"ping <#C4|quiet>"}]),
        ),
    ]);
    let analyzed = upload(&state, &app, &admin, &quiet).await?;
    let export_id = analyzed["id"].as_str().unwrap().to_string();
    for mappings in [
        json!([
            {"slack_channel_id":"C4","action":"create","name":format!("quiet-{suffix}"),"is_private":true},
            {"slack_channel_id":"C5","action":"skip"}
        ]),
        json!([
            {"slack_channel_id":"C4","action":"create","name":format!("quiet-{suffix}"),"is_private":false},
            {"slack_channel_id":"C5","action":"create","name":format!("loud-{suffix}")}
        ]),
    ] {
        let (status, body) = start(&app, &admin, &export_id, mappings).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        drive(
            &state,
            &app,
            &admin,
            body["id"].as_str().unwrap(),
            "completed",
        )
        .await?;
    }
    let rows = sql(
        &state,
        "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(
            &team,
            "C5",
            "1704430000.000200",
        )],
    )
    .await?;
    assert_eq!(rows[0]["content"], "ping #quiet");
    Ok(())
}

/// アップロード中に取り消したエクスポートは、取り消した後で PUT が終わっても、掃除で ZIP を消す
#[tokio::test]
async fn test_slack_import_removes_zip_uploaded_after_cancel() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_late").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }
    let zip = build_zip(&[(
        "users.json".into(),
        json!([{"id":"U1","team_id":"TLATE","name":"alice"}]),
    )]);
    let (status, created) = request(
        &app,
        "POST",
        "/api/admin/slack-imports",
        &admin,
        Some(json!({"file_name": "export.zip", "file_size": zip.len()})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["import"]["id"].as_str().unwrap().to_string();
    let key = format!("slack-imports/{id}/export.zip");
    let (status, cancelled) = request(
        &app,
        "DELETE",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    // 取り消した後で、ブラウザからの PUT が終わった
    let put = reqwest::Client::new()
        .put(created["upload_url"].as_str().unwrap())
        .header("content-type", "application/zip")
        .body(zip)
        .send()
        .await?;
    assert!(put.status().is_success());
    assert_s3_object_exists(&state.s3_client, &config.s3.bucket, &key, true).await?;

    // 署名付き URL の期限内は、まだ PUT が続いているかもしれないので消さない
    backend::slack::worker::cleanup(&state).await?;
    assert_s3_object_exists(&state.s3_client, &config.s3.bucket, &key, true).await?;
    // 期限と猶予を過ぎたら、もう一度だけ消す
    sql(
        &state,
        "UPDATE slack_import_jobs SET updated_at=now()-interval '30 days' WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&id],
    )
    .await?;
    backend::slack::worker::cleanup(&state).await?;
    assert_s3_object_exists(&state.s3_client, &config.s3.bucket, &key, false).await?;
    let job = sql(
        &state,
        "SELECT json_build_object('status',status,'key',storage_key)::text FROM slack_import_jobs WHERE id=$1",
        &[&id],
    )
    .await?;
    assert_eq!(job[0], json!({"status": "cancelled", "key": ""}));
    Ok(())
}

/// エクスポートのサイズでは上限内でも、取得したら上限を超えた添付だけの投稿は、ファイル名を本文に残す
#[tokio::test]
async fn test_slack_import_names_files_skipped_while_fetching() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let files = MockServer::start().await;
    for (name, body) in [("big1.bin", 64), ("big2.bin", 64), ("small.txt", 4)] {
        Mock::given(method("GET"))
            .and(path(format!("/files/{name}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_bytes(vec![b'x'; body]),
            )
            .mount(&files)
            .await;
    }
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = Some("xoxb-test".into());
    config.slack.files_base_url = format!("{}/files/", files.uri());
    config.s3.max_file_size = 16;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_skip").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let file = |name: &str| {
        json!({"id": format!("F-{name}"), "name": name, "mimetype": "text/plain",
               "url_private_download": format!("{}/files/{name}", files.uri())})
    };
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":format!("files-{suffix}"),"members":["U1"]}]),
        ),
        (
            format!("files-{suffix}/2024-01-01.json"),
            json!([
                // エクスポートにサイズがなく、取得すると上限を超える
                {"ts":"1704070000.000100","user":"U1","text":"","files":[file("big1.bin"), file("big2.bin")]},
                // 取得できる添付だけの投稿は、本文を空のままにする
                {"ts":"1704070000.000200","user":"U1","text":"","files":[file("small.txt")]},
                // 本文がある投稿はそのまま
                {"ts":"1704070000.000300","user":"U1","text":"keep","files":[file("big1.bin")]}
            ]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("files-{suffix}")}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let content = |ts: &'static str| {
        let (state, team) = (state.clone(), team.clone());
        async move {
            sql(
                &state,
                "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
                &[&backend::slack::message_id(&team, "C1", ts)],
            )
            .await
            .unwrap()[0]["content"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    // 取得の順で足すので、行の順には依らない
    let mut lines: Vec<String> = content("1704070000.000100")
        .await
        .lines()
        .map(str::to_string)
        .collect();
    lines.sort();
    assert_eq!(lines, vec!["big1.bin", "big2.bin"]);
    assert_eq!(content("1704070000.000200").await, "");
    assert_eq!(content("1704070000.000300").await, "keep");
    Ok(())
}

/// トークンなしで取り込んでファイル名を本文に入れた投稿は、後から取得できたら本文から外す。
/// サイズの上限でスキップした添付は、上限を上げて再取得できる。
#[tokio::test]
async fn test_slack_import_refetch_removes_names_and_retries_size_skips() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let files = MockServer::start().await;
    for name in ["late.txt", "big.bin", "zeta.txt", "alpha.txt", "report.pdf"] {
        Mock::given(method("GET"))
            .and(path(format!("/files/{name}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_bytes(vec![b'x'; 64]),
            )
            .mount(&files)
            .await;
    }
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    config.slack.files_base_url = format!("{}/files/", files.uri());
    let db = init_db(&config.database_url).await?;
    let no_token = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&no_token.s3_client, &config.s3.bucket).await;
    let app = create_router(no_token.clone());
    let admin = create_user(&app, "slack_admin_refetch").await?;
    sql(
        &no_token,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let file = |name: &str| {
        json!({"id": format!("F-{name}"), "name": name, "mimetype": "text/plain",
               "url_private_download": format!("{}/files/{name}", files.uri())})
    };
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":format!("refetch-{suffix}"),"members":["U1"]}]),
        ),
        (
            format!("refetch-{suffix}/2024-01-01.json"),
            json!([
                {"ts":"1704070000.000100","user":"U1","text":"","files":[file("late.txt")]},
                {"ts":"1704070000.000200","user":"U1","text":"","files":[file("big.bin")]},
                // 複数のファイル。Slack の files の順が名前の順と違っても、取得できたら本文から外す
                {"ts":"1704070000.000300","user":"U1","text":"","files":[file("zeta.txt"), file("alpha.txt")]},
                // 投稿者が書いた本文が、添付のファイル名と同じ。取得できても、本文は消さない
                {"ts":"1704070000.000400","user":"U1","text":"report.pdf","files":[file("report.pdf")]}
            ]),
        ),
    ]);
    let export = upload(&no_token, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("refetch-{suffix}")}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    drive(&no_token, &app, &admin, &run, "completed").await?;
    let content = |state: AppState, ts: &'static str| {
        let team = team.clone();
        async move {
            sql(
                &state,
                "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
                &[&backend::slack::message_id(&team, "C1", ts)],
            )
            .await
            .unwrap()[0]["content"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    // トークンがないので、どちらも保留のまま、ファイル名が本文に入る
    assert_eq!(
        content(no_token.clone(), "1704070000.000100").await,
        "late.txt"
    );
    assert_eq!(
        content(no_token.clone(), "1704070000.000200").await,
        "big.bin"
    );
    assert_eq!(
        content(no_token.clone(), "1704070000.000300").await,
        "zeta.txt\nalpha.txt"
    );

    // トークンを設定し、上限を小さくして取得すると、big.bin は上限でスキップされ、late.txt も上限でスキップされる
    let mut small = config.clone();
    small.slack.import_bot_token = Some("xoxb-test".into());
    small.s3.max_file_size = 16;
    let small_state = AppState::new(db.clone(), small.clone());
    let app_small = create_router(small_state.clone());
    let (status, body) = request(
        &app_small,
        "POST",
        &format!("/api/admin/slack-imports/{run}/fetch-files"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(&small_state, &app_small, &admin, &run, "completed").await?;
    // 上限が原因のスキップは、再取得の対象として数える
    let (_, job) = request(
        &app_small,
        "GET",
        &format!("/api/admin/slack-imports/{run}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(job["skipped_files"], 5, "{job}");
    // 再取得できるスキップが残るジョブは、新しい履歴が増えても一覧から押し出されない
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement(
            "INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id,source_id) \
             SELECT 'history-skip-'||g,created_by,'completed',file_name,file_size,storage_key,team_id,source_id \
             FROM slack_import_jobs, generate_series(1,60) g WHERE id=$1",
        )
        .bind(run.as_str())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }
    let (_, listed) = request(&app_small, "GET", "/api/admin/slack-imports", &admin, None).await?;
    let still_listed = listed["imports"]
        .as_array()
        .unwrap()
        .iter()
        .any(|j| j["id"] == run.as_str());
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("DELETE FROM slack_import_jobs WHERE id LIKE 'history-skip-%'")
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
    }
    assert!(
        still_listed,
        "a job with retryable skipped files stays listed"
    );

    // 上限を戻して再取得すると、取得できて、本文からファイル名が外れる
    let mut big = config.clone();
    big.slack.import_bot_token = Some("xoxb-test".into());
    let big_state = AppState::new(db.clone(), big);
    let app_big = create_router(big_state.clone());
    let (status, body) = request(
        &app_big,
        "POST",
        &format!("/api/admin/slack-imports/{run}/fetch-files"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(&big_state, &app_big, &admin, &run, "completed").await?;
    assert_eq!(content(big_state.clone(), "1704070000.000100").await, "");
    assert_eq!(content(big_state.clone(), "1704070000.000200").await, "");
    assert_eq!(content(big_state.clone(), "1704070000.000300").await, "");
    assert_eq!(
        content(big_state.clone(), "1704070000.000400").await,
        "report.pdf",
        "the poster's own text stays even when it equals the file name"
    );
    let attachments = sql(
        &big_state,
        "SELECT json_build_object('n',count(*))::text FROM attachments WHERE message_id IN ($1,$2)",
        &[
            &backend::slack::message_id(&team, "C1", "1704070000.000100"),
            &backend::slack::message_id(&team, "C1", "1704070000.000300"),
        ],
    )
    .await?;
    assert_eq!(attachments[0]["n"], 3);
    Ok(())
}

/// 同じファイル名の添付が複数あり、片方だけ取得できたときは、取得できなかった側の名前を本文に残す。
#[tokio::test]
async fn test_slack_import_same_name_files_keep_the_unfetched_name() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let files = MockServer::start().await;
    for (dir, size) in [("a", 8usize), ("b", 64usize)] {
        Mock::given(method("GET"))
            .and(path(format!("/files/{dir}/image.png")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(vec![b'x'; size]),
            )
            .mount(&files)
            .await;
    }
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    config.slack.files_base_url = format!("{}/files/", files.uri());
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_samename").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let file = |dir: &str| {
        json!({"id": format!("F-{dir}"), "name": "image.png", "mimetype": "image/png",
               "url_private_download": format!("{}/files/{dir}/image.png", files.uri())})
    };
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":format!("samename-{suffix}"),"members":["U1"]}]),
        ),
        (
            format!("samename-{suffix}/2024-01-01.json"),
            json!([
                {"ts":"1704070000.000100","user":"U1","text":"","files":[file("a"), file("b")]}
            ]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("samename-{suffix}")}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    drive(&state, &app, &admin, &run, "completed").await?;
    let message = backend::slack::message_id(&team, "C1", "1704070000.000100");
    let body = |state: AppState| {
        let message = message.clone();
        async move {
            sql(
                &state,
                "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
                &[&message],
            )
            .await
            .unwrap()[0]["content"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    // トークンがないので、2 件とも保留のまま、同じ名前が 2 行入る
    assert_eq!(body(state.clone()).await, "image.png\nimage.png");
    // トークンを設定し、上限を小さくして取得すると、a (8 バイト) は取得でき、b (64 バイト) は上限でスキップされる
    let mut small = config.clone();
    small.slack.import_bot_token = Some("xoxb-test".into());
    small.s3.max_file_size = 16;
    let small_state = AppState::new(db.clone(), small);
    let app_small = create_router(small_state.clone());
    let (status, fetched) = request(
        &app_small,
        "POST",
        &format!("/api/admin/slack-imports/{run}/fetch-files"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    drive(&small_state, &app_small, &admin, &run, "completed").await?;
    assert_eq!(
        body(small_state.clone()).await,
        "image.png",
        "one image.png fetched and one skipped leaves one name in the body"
    );
    let attachments = sql(
        &small_state,
        "SELECT json_build_object('n',count(*))::text FROM attachments WHERE message_id=$1",
        &[&message],
    )
    .await?;
    assert_eq!(attachments[0]["n"], 1);
    Ok(())
}

/// 取り込み先は、新しく作ったチャンネルも既存のチャンネルも「Slack から取り込んだ」印が付く。
/// 新しく作るチャンネルは、指定があれば取り込みの完了時にアーカイブされる (既存のチャンネルはアーカイブしない)。
#[tokio::test]
async fn test_slack_import_flags_targets_and_archives_new_channels_on_request() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_flag").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let existing_id = uuid::Uuid::new_v4().to_string();
    let existing_name = format!("flag-existing-{suffix}");
    sql(
        &state,
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,false) RETURNING json_build_object('ok',true)::text",
        &[&existing_id, &existing_name, &admin],
    )
    .await?;
    let day = |ts: &str| json!([{"ts": ts, "user": "U1", "text": "hello"}]);
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([
                {"id":"C1","name":format!("flag-keep-{suffix}"),"members":["U1"]},
                {"id":"C2","name":format!("flag-archive-{suffix}"),"members":["U1"]},
                {"id":"C3","name":format!("flag-into-{suffix}"),"members":["U1"]},
                {"id":"C4","name":format!("flag-skip-{suffix}"),"members":["U1"]}
            ]),
        ),
        (
            format!("flag-keep-{suffix}/2024-01-01.json"),
            day("1704070000.000100"),
        ),
        (
            format!("flag-archive-{suffix}/2024-01-01.json"),
            day("1704070000.000100"),
        ),
        (
            format!("flag-into-{suffix}/2024-01-01.json"),
            day("1704070000.000100"),
        ),
        (
            format!("flag-skip-{suffix}/2024-01-01.json"),
            day("1704070000.000100"),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([
            {"slack_channel_id":"C1","action":"create","name":format!("flag-keep-{suffix}")},
            {"slack_channel_id":"C2","action":"create","name":format!("flag-archive-{suffix}"),"archive":true},
            {"slack_channel_id":"C3","action":"existing","channel_id":existing_id},
            {"slack_channel_id":"C4","action":"skip"}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let row = |name: String| {
        let state = state.clone();
        async move {
            sql(
                &state,
                "SELECT json_build_object('flag',imported_from_slack,'archived',archive_at IS NOT NULL,\
                 'kind',slack_import_kind,'at',slack_imported_at IS NOT NULL)::text \
                 FROM channels WHERE channel_name=$1",
                &[&name],
            )
            .await
            .unwrap()
        }
    };
    let keep = row(format!("flag-keep-{suffix}")).await;
    assert_eq!(
        (&keep[0]["flag"], &keep[0]["archived"]),
        (&json!(true), &json!(false))
    );
    let archived = row(format!("flag-archive-{suffix}")).await;
    assert_eq!(
        (&archived[0]["flag"], &archived[0]["archived"]),
        (&json!(true), &json!(true)),
        "a new channel asked to be archived is archived once the import completes"
    );
    let existing = row(existing_name).await;
    assert_eq!(
        (&existing[0]["flag"], &existing[0]["archived"]),
        (&json!(true), &json!(false)),
        "an existing target is flagged but never archived"
    );
    assert!(
        row(format!("flag-skip-{suffix}")).await.is_empty(),
        "skipped channels are not created"
    );
    // 取り込みの種類 (新しく作った / 既存のチャンネルへの追加) と、取り込んだ日時が付く
    for (channel, kind) in [
        (&keep, "created"),
        (&archived, "created"),
        (&existing, "existing"),
    ] {
        assert_eq!(channel[0]["kind"], kind);
        assert_eq!(channel[0]["at"], true);
    }
    // 取り込み先に固定された再取り込みでは、種類は最初のまま (created) で、日時だけが更新される
    let stamp = |name: String| {
        let state = state.clone();
        async move {
            sql(
                &state,
                "SELECT json_build_object('id',id,'at',slack_imported_at::text)::text FROM channels WHERE channel_name=$1",
                &[&name],
            )
            .await
            .unwrap()
        }
    };
    let before = stamp(format!("flag-keep-{suffix}")).await;
    let (status, rerun) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":before[0]["id"]}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{rerun}");
    drive(
        &state,
        &app,
        &admin,
        rerun["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let after = stamp(format!("flag-keep-{suffix}")).await;
    assert_ne!(
        before[0]["at"], after[0]["at"],
        "the import time is updated"
    );
    let again = row(format!("flag-keep-{suffix}")).await;
    assert_eq!(again[0]["kind"], "created", "the first kind is kept");
    // 管理者の取り込み画面に、以前の取り込み先の種類と最後に取り込んだ日時を返す
    let (status, saved) = request(
        &app,
        "GET",
        &format!(
            "/api/admin/slack-imports/{}",
            export["id"].as_str().unwrap()
        ),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let channels = saved["analysis"]["channels"].as_array().unwrap();
    let c1 = channels
        .iter()
        .find(|channel| channel["slack_channel_id"] == "C1")
        .unwrap();
    assert_eq!(c1["existing_channel"]["import_kind"], "created");
    assert!(c1["existing_channel"]["imported_at"].is_string());
    Ok(())
}

/// マイグレーションの補完 (取り込みの種類) は、ジョブの最終状態に関わらず、「新しいチャンネル」の指定があれば
/// created にする。ジョブが途中で失敗・取り消しになっても、作ったチャンネルと対応表は先にコミットされて残る。
/// ロールバックするトランザクションの中で、実際のマイグレーションの補完の文を実行して確かめる。
#[tokio::test]
async fn test_slack_import_info_backfill_ignores_the_final_job_status() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_backfill").await?;
    let migration =
        include_str!("../../infra/db/migrations/20261112000000_channels_slack_import_info.sql");
    let backfill = &migration[migration
        .find("UPDATE \"channels\"")
        .expect("the backfill statement")..];

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // (ジョブの状態, 指定, 取り込み先が「新しいチャンネル」の決定的な ID か, 期待する種類, 取り込みの日時が入るか)
    let cases = [
        ("completed", "create", true, "created", true),
        ("fetching_files", "create", true, "created", true),
        ("failed", "create", true, "created", false),
        ("cancelled", "create", true, "created", false),
        ("completed", "existing", false, "existing", true),
        // 作成の指定のジョブがチャンネルを作る前に失敗し、あとで既存のチャンネルに取り込んだ
        ("failed", "create", false, "existing", false),
    ];
    let mut channel_ids = Vec::new();
    for (index, (status, action, deterministic, _, _)) in cases.iter().enumerate() {
        let slack_channel = format!("C{index}");
        let channel_id = if *deterministic {
            backend::slack::channel_id(&team, &slack_channel)
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        query(
            &mut tx,
            "INSERT INTO channels(id,channel_name,created_user_id,is_private,imported_from_slack) \
             VALUES($1,$2,$3,false,true) RETURNING json_build_object('ok',true)::text",
            &[&channel_id, &format!("backfill-{index}-{suffix}"), &admin],
        )
        .await?;
        query(
            &mut tx,
            "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id) VALUES($1,$2,$3) \
             RETURNING json_build_object('ok',true)::text",
            &[&team, &slack_channel, &channel_id],
        )
        .await?;
        let mappings = json!([{"slack_channel_id": slack_channel, "action": action,
            "name": format!("backfill-{index}-{suffix}"), "channel_id": channel_id}])
        .to_string();
        query(
            &mut tx,
            "INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id,mappings) \
             VALUES($1,$2,$3,'export.zip',1,'k',$4,$5) RETURNING json_build_object('ok',true)::text",
            &[&format!("backfill-job-{index}-{suffix}"), &admin, status, &team, &mappings],
        )
        .await?;
        channel_ids.push(channel_id);
    }
    toasty::sql::statement(backfill).exec(&mut tx).await?;
    for ((status, action, _, expected, imported), channel_id) in cases.iter().zip(&channel_ids) {
        let rows = query(
            &mut tx,
            "SELECT json_build_object('kind',slack_import_kind,'at',slack_imported_at IS NOT NULL)::text \
             FROM channels WHERE id=$1",
            &[channel_id],
        )
        .await?;
        assert_eq!(rows[0]["kind"], *expected, "{action} job in {status}");
        assert_eq!(
            rows[0]["at"], *imported,
            "{action} job in {status}: the import time comes only from a job that finished importing"
        );
    }
    // コミットしない (ロールバックされる)
    drop(tx);
    Ok(())
}

/// 編集・削除の履歴を含むエクスポートの記録 (`message_changed` / `message_deleted`) は、投稿として取り込まず、
/// 以前の取り込みで入れてしまった分 (編集した日時の重複した投稿、削除された元の投稿) は、再取り込みで論理削除する。
#[tokio::test]
async fn test_slack_import_skips_history_events_and_removes_stale_rows() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_history").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("history-{suffix}");
    let (a, edit, b, delete, c) = (
        "1704070000.000100",
        "1704070100.000000",
        "1704070200.000100",
        "1704070250.000000",
        "1704070300.000100",
    );
    let zip = |messages: Value| {
        build_zip(&[
            (
                "users.json".into(),
                json!([{"id":"U1","team_id":team,"name":"alice"}]),
            ),
            (
                "channels.json".into(),
                json!([{"id":"C1","name":name,"members":["U1"]}]),
            ),
            (format!("{name}/2024-01-01.json"), messages),
        ])
    };
    // 以前の版の取り込み: 編集の記録を投稿として入れ (編集した日時の重複した投稿)、削除される前の投稿も入れていた
    let before = zip(json!([
        {"ts": a, "user": "U1", "text": "A"},
        {"ts": edit, "user": "U1", "text": "A edited"},
        {"ts": b, "user": "U1", "text": "B before it was deleted"},
        {"ts": c, "user": "U1", "text": "C"}
    ]));
    let export = upload(&state, &app, &admin, &before).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":name}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let deleted = |ts: &'static str| {
        let state = state.clone();
        let id = backend::slack::message_id(&team, "C1", ts);
        async move {
            sql(
                &state,
                "SELECT json_build_object('deleted',deleted_at IS NOT NULL)::text FROM messages WHERE id=$1",
                &[&id],
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(
        deleted(edit).await[0]["deleted"],
        false,
        "the old version imported it"
    );

    // 編集・削除の履歴を含むエクスポート: 元の投稿 (A, C) と、編集・削除の記録 (B の元の記録は残らない)
    let after = zip(json!([
        {"ts": a, "user": "U1", "text": "A edited"},
        {"type":"message","subtype":"message_changed","ts": edit, "user": "U1", "text": "A edited",
         "edited_by": "U1", "original": {"user": "U1", "type": "message", "ts": a, "text": "A"}},
        {"type":"message","subtype":"message_deleted","ts": delete, "user": "U1", "text": "",
         "deleted_by": "U1", "original": {"user": "U1", "type": "message", "ts": b, "text": "B before it was deleted"}},
        {"ts": c, "user": "U1", "text": "C"}
    ]));
    let export = upload(&state, &app, &admin, &after).await?;
    // 解析のメッセージ数に、編集・削除の記録を含めない
    assert_eq!(
        export["analysis"]["channels"][0]["message_count"], 2,
        "{export}"
    );
    let target = backend::slack::channel_id(&team, "C1");
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":target}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    assert_eq!(
        deleted(a).await[0]["deleted"],
        false,
        "the original post stays"
    );
    assert_eq!(deleted(c).await[0]["deleted"], false, "other posts stay");
    assert_eq!(
        deleted(edit).await[0]["deleted"],
        true,
        "the duplicate post made from the edit record is removed"
    );
    assert_eq!(
        deleted(b).await[0]["deleted"],
        true,
        "the post that was deleted in Slack is removed"
    );
    assert!(
        deleted(delete).await.is_empty(),
        "the delete record itself is never imported"
    );
    // 生きている投稿は、元の投稿 (A と C) だけ
    let live = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND deleted_at IS NULL",
        &[&target],
    )
    .await?;
    assert_eq!(live[0]["n"], 2);
    Ok(())
}

/// 同じ日付ファイルに、削除された元の投稿の普通の記録と、削除の記録がある場合は、取り込んでから論理削除する
/// (初回と再取り込みで結果が同じ。その投稿への返信は、削除された親のスレッドに残る)。
/// 公開されている形式 (`original_ts`) の削除の記録も扱う。イベントの `ts` と同じ `ts` の本物の投稿は消さない。
#[tokio::test]
async fn test_slack_import_history_events_in_the_same_file() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_history_same").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let (name, other) = (format!("same-{suffix}"), format!("other-{suffix}"));
    let (a, b, x, delete) = (
        "1704070000.000100",
        "1704070200.000100",
        "1704070100.000000",
        "1704070250.000000",
    );
    // `reply` は、削除された B への返信。`d` は、公開されている形式 (`original_ts`) の削除の記録で消える
    let (reply, d, delete_d) = (
        "1704070210.000100",
        "1704070300.000100",
        "1704070350.000000",
    );
    // `x` は、本物の投稿の `ts` が、編集の記録の `ts` と同じ (秒ちょうど) になった場合
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([
                {"id":"C1","name":name,"members":["U1"]},
                {"id":"C2","name":other,"members":["U1"]}
            ]),
        ),
        (
            format!("{name}/2024-01-01.json"),
            json!([
                {"ts": a, "user": "U1", "text": "A"},
                {"ts": x, "user": "U1", "text": "X is a real post"},
                {"type":"message","subtype":"message_changed","ts": x, "user": "U1", "text": "edit",
                 "original": {"user": "U1", "type": "message", "ts": a, "text": "A"}},
                {"ts": b, "user": "U1", "text": "B was deleted", "thread_ts": b},
                {"ts": reply, "user": "U1", "text": "reply to B", "thread_ts": b},
                {"ts": d, "user": "U1", "text": "D was deleted"},
                {"type":"message","subtype":"message_deleted","ts": delete_d, "user": "U1", "text": "",
                 "original_ts": d, "previous": {"user": "U1", "type": "message", "ts": d, "text": "D was deleted"}},
                {"type":"message","subtype":"message_deleted","ts": delete, "user": "U1", "text": "",
                 "original": {"user": "U1", "type": "message", "ts": b, "text": "B was deleted"}}
            ]),
        ),
        // 別のチャンネルに、同じ `ts` の本物の投稿がある (削除の記録は C1 のものだけ)
        (
            format!("{other}/2024-01-01.json"),
            json!([{"ts": b, "user": "U1", "text": "B in another channel"}]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let mappings = json!([
        {"slack_channel_id":"C1","action":"create","name":name},
        {"slack_channel_id":"C2","action":"create","name":other}
    ]);
    let (status, run) = start(&app, &admin, export["id"].as_str().unwrap(), mappings).await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let live = |channel: &'static str| {
        let state = state.clone();
        let channel_id = backend::slack::channel_id(&team, channel);
        async move {
            sql(
                &state,
                "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND deleted_at IS NULL",
                &[&channel_id],
            )
            .await
            .unwrap()[0]["n"]
                .as_i64()
                .unwrap()
        }
    };
    let row = |channel: &'static str, ts: &'static str| {
        let state = state.clone();
        let id = backend::slack::message_id(&team, channel, ts);
        async move {
            sql(
                &state,
                "SELECT json_build_object('deleted',deleted_at IS NOT NULL)::text FROM messages WHERE id=$1",
                &[&id],
            )
            .await
            .unwrap()
        }
    };
    // 削除された B と D は、初回の取り込みから論理削除される。A と、同じ `ts` の本物の投稿 X、B への返信は残る
    assert_eq!(live("C1").await, 3);
    assert_eq!(row("C1", b).await[0]["deleted"], true);
    assert_eq!(row("C1", d).await[0]["deleted"], true, "original_ts format");
    // 返信は、削除された親のスレッドに残る (チャンネル直下に出ない)
    let reply_row = sql(
        &state,
        "SELECT json_build_object('parent',parent_id,'deleted',deleted_at IS NOT NULL)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(&team, "C1", reply)],
    )
    .await?;
    assert_eq!(
        reply_row[0]["parent"],
        backend::slack::message_id(&team, "C1", b)
    );
    assert_eq!(reply_row[0]["deleted"], false);
    assert_eq!(row("C1", x).await[0]["deleted"], false);
    assert_eq!(
        row("C2", b).await[0]["deleted"],
        false,
        "another channel is untouched"
    );

    // 再取り込みしても、同じ (X は、イベントと同じ `ts` でも消えない)
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([
            {"slack_channel_id":"C1","action":"existing","channel_id":backend::slack::channel_id(&team, "C1")},
            {"slack_channel_id":"C2","action":"existing","channel_id":backend::slack::channel_id(&team, "C2")}
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    assert_eq!(live("C1").await, 3, "re-importing changes nothing");
    assert_eq!(row("C1", b).await[0]["deleted"], true);
    assert_eq!(row("C1", x).await[0]["deleted"], false);
    assert_eq!(live("C2").await, 1);
    Ok(())
}

/// Slack の `url_private_download` が 302 を返しても、許可した URL なら追従して取得する。
/// Authorization は同じオリジンにだけ付け、許可していない URL へのリダイレクトは追従しない。
#[tokio::test]
async fn test_slack_import_follows_allowed_redirects_only() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let files = MockServer::start().await;
    let other = MockServer::start().await;
    // 同じオリジンの 302 → 許可した URL。トークンは両方に付く
    Mock::given(method("GET"))
        .and(path("/files/hop.txt"))
        .and(header("authorization", "Bearer xoxb-test"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/files/real.txt"))
        .mount(&files)
        .await;
    Mock::given(method("GET"))
        .and(path("/files/real.txt"))
        .and(header("authorization", "Bearer xoxb-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_bytes(b"followed".to_vec()),
        )
        .mount(&files)
        .await;
    // 許可していない別のオリジンへの 302 は追従しない (トークンも送らない)
    Mock::given(method("GET"))
        .and(path("/files/away.txt"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/steal.txt", other.uri())),
        )
        .mount(&files)
        .await;
    Mock::given(method("GET"))
        .and(path("/steal.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"leaked".to_vec()))
        .expect(0)
        .mount(&other)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = Some("xoxb-test".into());
    config.slack.files_base_url = format!("{}/files/", files.uri());
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_redirect").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let file = |name: &str| {
        json!({"id": format!("F-{name}"), "name": name, "mimetype": "text/plain", "size": 8,
               "url_private_download": format!("{}/files/{name}", files.uri())})
    };
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":format!("redir-{suffix}"),"members":["U1"]}]),
        ),
        (
            format!("redir-{suffix}/2024-01-01.json"),
            json!([
                {"ts":"1704070000.000100","user":"U1","text":"a","files":[file("hop.txt")]},
                {"ts":"1704070000.000200","user":"U1","text":"b","files":[file("away.txt")]}
            ]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("redir-{suffix}")}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    let status_of = |ts: &'static str| {
        let (state, team) = (state.clone(), team.clone());
        async move {
            sql(
                &state,
                "SELECT json_build_object('s',f.status) ::text FROM slack_files f WHERE f.message_id=$1",
                &[&backend::slack::message_id(&team, "C1", ts)],
            )
            .await
            .unwrap()[0]["s"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    assert_eq!(status_of("1704070000.000100").await, "done");
    // 許可していない URL へのリダイレクトは追従しない (取得できず、別のオリジンへはトークンも送らない)
    assert_ne!(status_of("1704070000.000200").await, "done");
    Ok(())
}

/// 予約語や既存の名前と重なるときの `slack-` 付きの候補が、すでに使われていれば、別の名前を候補にする
#[tokio::test]
async fn test_slack_import_suggested_name_avoids_a_taken_prefixed_name() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_suggest").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    // `general-{suffix}` と `slack-general-{suffix}` が、どちらも Zoo にある
    for name in [
        format!("general-{suffix}"),
        format!("slack-general-{suffix}"),
    ] {
        let (status, _) = request(
            &app,
            "POST",
            "/api/channels",
            &admin,
            Some(json!({"id": uuid::Uuid::new_v4().to_string(), "channel_name": name, "is_private": false})),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":format!("general-{suffix}"),"members":["U1"]}]),
        ),
        (
            format!("general-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000100","user":"U1","text":"hi"}]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let suggested = export["analysis"]["channels"][0]["suggested_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert_ne!(suggested, format!("slack-general-{suffix}"), "{export}");
    assert!(suggested.starts_with("slack-general-"), "{suggested}");
    assert!(suggested.chars().count() <= 80, "{suggested}");
    Ok(())
}

/// 決定的な ID のチャンネルを他人が先に作っていても、取り込みはそれを再利用しない。
/// 名前も、ジョブの失敗理由やエラーに出さない。
#[tokio::test]
async fn test_import_does_not_reuse_a_foreign_channel_with_the_deterministic_id()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.team_id = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_foreign").await?;
    let alice = create_user(&app, "slack_alice_foreign").await?;
    let mallory = create_user(&app, "slack_mallory").await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    sql(
        &state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U1','x') \
         RETURNING json_build_object('ok',true)::text",
        &[&alice, &team],
    )
    .await?;
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C7","name":format!("mine-{suffix}"),"creator":"U1","members":["U1"]}]),
        ),
        (
            format!("mine-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000100","user":"U1","text":"private history"}]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    // mallory が、決定的な ID のチャンネルを先に作り、alice を入れておく
    let foreign = backend::slack::channel_id(&team, "C7");
    let (status, _) = request(
        &app,
        "POST",
        "/api/channels",
        &mallory,
        Some(
            json!({"id": foreign, "channel_name": format!("secret-{suffix}"), "is_private": true}),
        ),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    sql(
        &state,
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) RETURNING json_build_object('ok',true)::text",
        &[&alice, &foreign],
    )
    .await?;

    // ワーカーが、前回の取り込み先として記録されていないチャンネルを再利用せず、失敗理由に名前を出さない
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C7","action":"create","name":format!("mine-{suffix}")}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    let mut job = Value::Null;
    for _ in 0..100 {
        backend::slack::worker::run_once(&state).await?;
        job = request(
            &app,
            "GET",
            &format!("/api/admin/slack-imports/{run}"),
            &admin,
            None,
        )
        .await?
        .1;
        if job["status"] == "failed" {
            break;
        }
    }
    assert_eq!(job["status"], "failed", "{job}");
    assert!(
        job["error"]
            .as_str()
            .unwrap_or_default()
            .contains("same ID"),
        "{job}"
    );
    let imported = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1",
        &[&foreign],
    )
    .await?;
    assert_eq!(imported[0]["n"], 0);
    Ok(())
}

/// 決定的な ID のメッセージを他人が先に作っていても、取り込みはその行に投稿・リアクションを結び付けない。
/// 取り込みで作った行は、これまでどおり何度取り込んでも重複しない。
#[tokio::test]
async fn test_import_does_not_adopt_a_message_that_uses_the_deterministic_id() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.team_id = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_squat").await?;
    let mallory = create_user(&app, "slack_mallory_squat").await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let name = format!("squat-{suffix}");
    let zip = |messages: Value| {
        build_zip(&[
            (
                "users.json".into(),
                json!([{"id":"U1","team_id":team,"name":"alice"}]),
            ),
            (
                "channels.json".into(),
                json!([{"id":"C9","name":name,"members":["U1"]}]),
            ),
            (format!("{name}/2024-01-01.json"), messages),
        ])
    };
    let first = json!({"ts":"1704070000.000100","user":"U1","text":"first"});
    let second = json!({"ts":"1704070100.000200","user":"U1","text":"second",
        "reactions":[{"name":"thumbsup","users":["U1"],"count":1}]});

    let export = upload(&state, &app, &admin, &zip(json!([first]))).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C9","action":"create","name":name}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;

    // 取り込み先に書ける人が、まだ取り込んでいない投稿の ID を先に使う
    let channel = backend::slack::channel_id(&team, "C9");
    let squatted = backend::slack::message_id(&team, "C9", "1704070100.000200");
    sql(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'fake') \
         RETURNING json_build_object('ok',true)::text",
        &[&squatted, &mallory, &channel],
    )
    .await?;

    // 返信の ID も先に使われる。親を後から取り込んでも、その行はスレッドに移らない
    let reply_squat = backend::slack::message_id(&team, "C9", "1704070300.000400");
    sql(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'fake reply') \
         RETURNING json_build_object('ok',true)::text",
        &[&reply_squat, &mallory, &channel],
    )
    .await?;
    let parent = json!({"ts":"1704070200.000300","user":"U1","text":"parent","thread_ts":"1704070200.000300",
        "replies":[{"user":"U1","ts":"1704070300.000400"}]});
    let reply = json!({"ts":"1704070300.000400","user":"U1","text":"reply","thread_ts":"1704070200.000300"});
    let export = upload(
        &state,
        &app,
        &admin,
        &zip(json!([first, second, parent, reply])),
    )
    .await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C9","action":"create","name":name}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;

    let rows = sql(
        &state,
        "SELECT json_build_object('content',content,'user_id',user_id)::text FROM messages WHERE id=$1",
        &[&squatted],
    )
    .await?;
    assert_eq!(rows[0]["content"], "fake");
    assert_eq!(rows[0]["user_id"], mallory.as_str());
    let reactions = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM message_reactions WHERE message_id=$1",
        &[&squatted],
    )
    .await?;
    assert_eq!(reactions[0]["n"], 0);
    let threaded = sql(
        &state,
        "SELECT json_build_object('p',parent_id)::text FROM messages WHERE id=$1",
        &[&reply_squat],
    )
    .await?;
    assert!(threaded[0]["p"].is_null(), "{threaded:?}");
    // 取り込みで作った最初の投稿は、再取り込みでも重複しない
    let total = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1",
        &[&channel],
    )
    .await?;
    // 最初の投稿、先に使われた 2 件、親 (返信は飛ばす)
    assert_eq!(total[0]["n"], 4, "{total:?}");
    Ok(())
}

#[tokio::test]
async fn test_slack_import_labels_files_in_existing_nsfw_channel() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/F1/photo.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3, 4]))
        .mount(&files)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = Some("xoxb-test".into());
    config.slack.files_base_url = format!("{}/files/", files.uri());
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "nsfw_import_admin").await?;
    let mut db = db.clone();
    toasty::sql::statement("INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin')")
        .bind(admin.as_str())
        .exec(&mut db)
        .await?;
    let channel = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &app,
        "POST",
        "/api/channels",
        &admin,
        Some(json!({"id":channel,"channel_name":channel,"is_nsfw":true})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let zip = build_zip(&[
        ("team.json".into(), json!({"id":team,"name":"NSFW import"})),
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice","profile":{"real_name":"Alice"}}]),
        ),
        (
            "channels.json".into(),
            json!([{"id":"C1","name":"photos","members":["U1"]}]),
        ),
        (
            "photos/2024-01-01.json".into(),
            json!([{"ts":"1704070000.000100","user":"U1","text":"photo","files":[{"id":"F1","name":"photo.png","mimetype":"image/png","size":4,"url_private_download":format!("{}/files/F1/photo.png",files.uri())}]}]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":channel}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    let result = drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await?;
    assert_eq!(result["failed_files"], 0);
    let attachments = sql(&state, "SELECT json_build_object('is_sensitive',is_sensitive)::text FROM attachments WHERE channel_id=$1", &[&channel]).await?;
    assert_eq!(attachments, vec![json!({"is_sensitive":true})]);
    Ok(())
}

/// 取り込みを始め、ワーカーが取る前の `importing` のジョブを返す (テストの準備)
async fn importing_job(
    state: &AppState,
    app: &Router,
    admin: &str,
    team: &str,
    name: &str,
    days: &[(&str, Value)],
) -> anyhow::Result<String> {
    let mut files = vec![
        (
            "users.json".to_string(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".to_string(),
            json!([{"id":"C1","name":name,"members":["U1"]}]),
        ),
    ];
    for (day, messages) in days {
        files.push((format!("{name}/{day}.json"), messages.clone()));
    }
    let export = upload(state, app, admin, &build_zip(&files)).await?;
    let (status, run) = start(
        app,
        admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":name}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    Ok(run["id"].as_str().unwrap().to_string())
}

async fn admin_state() -> anyhow::Result<(AppState, Router, String)> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_handoff").await?;
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    Ok((state, app, admin))
}

/// 終了の合図でジョブを手放しても、attempts を消費せず、別のワーカーが続きを完了できる
#[tokio::test]
async fn test_slack_import_releases_the_job_on_shutdown() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let (state, app, admin) = admin_state().await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let name = format!("handoff-{}", uuid::Uuid::new_v4().simple());
    let id = importing_job(
        &state,
        &app,
        &admin,
        &team,
        &name,
        &[(
            "2024-01-01",
            json!([{"ts":"1704070000.000100","user":"U1","text":"hello"}]),
        )],
    )
    .await?;
    let job = |state: &AppState| {
        let state = state.clone();
        let id = id.clone();
        async move {
            sql(
                &state,
                "SELECT json_build_object('attempts',attempts,'handoffs',handoffs,'token',lease_token,\
                 'waiting',available_at>now(),'status',status)::text FROM slack_import_jobs WHERE id=$1",
                &[&id],
            )
            .await
            .unwrap()
            .remove(0)
        }
    };

    // すでに合図が来ているワーカーは、ジョブを取らない
    let (stop_tx, mut stop) = tokio::sync::watch::channel(true);
    assert!(matches!(
        backend::slack::worker::run_once_until(&state, &mut stop).await?,
        backend::slack::worker::Step::Stopped
    ));
    assert_eq!(job(&state).await["attempts"], 0);
    drop(stop_tx);

    // ワーカーが取って (attempts が増え、リースを持つ)、処理中に落ちる直前の状態を作る
    sql(
        &state,
        "UPDATE slack_import_jobs SET attempts=3,lease_token='tok',lease_until=now()+interval '2 minutes' \
         WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&id],
    )
    .await?;
    // 別のトークン (リースを奪われた) では、何もしない
    assert!(!backend::slack::worker::release_job(&state, &id, "other").await?);
    assert_eq!(job(&state).await["attempts"], 3);

    assert!(backend::slack::worker::release_job(&state, &id, "tok").await?);
    let released = job(&state).await;
    assert_eq!(
        released["attempts"], 2,
        "the hand-off gives the attempt back"
    );
    assert_eq!(released["handoffs"], 1);
    assert!(released["token"].is_null(), "{released}");
    assert_eq!(released["waiting"], true, "not taken again right away");
    // 2 回目の手放しは、トークンが消えているので何もしない (attempts を二重に戻さない)
    assert!(!backend::slack::worker::release_job(&state, &id, "tok").await?);
    assert_eq!(job(&state).await["attempts"], 2);

    // 待ち時間が過ぎれば、別のワーカーが取って完了する
    sql(
        &state,
        "UPDATE slack_import_jobs SET available_at=now() WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&id],
    )
    .await?;
    drive(&state, &app, &admin, &id, "completed").await?;
    Ok(())
}

/// 前の試行が取り込んだ日付ファイルは飛ばし、続きから取り込む。処理済みの件数も引き継ぐ
#[tokio::test]
async fn test_slack_import_resumes_after_the_completed_day_files() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let (state, app, admin) = admin_state().await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let name = format!("resume-{}", uuid::Uuid::new_v4().simple());
    let (d1, d2, d3) = (
        "1704070000.000100",
        "1704156400.000100",
        "1704242800.000100",
    );
    let id = importing_job(
        &state,
        &app,
        &admin,
        &team,
        &name,
        &[
            ("2024-01-01", json!([{"ts":d1,"user":"U1","text":"day 1"}])),
            ("2024-01-02", json!([{"ts":d2,"user":"U1","text":"day 2"}])),
            ("2024-01-03", json!([{"ts":d3,"user":"U1","text":"day 3"}])),
        ],
    )
    .await?;
    // 前の試行が 1 日目まで取り込んでコミットした状態 (1 日目の投稿は、ここでは入れない。飛ばされることの確認)
    let progress = json!({"C1": format!("{name}/2024-01-01.json")}).to_string();
    sql(
        &state,
        "UPDATE slack_import_jobs SET import_progress=$2,processed_messages=1 WHERE id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&id, &progress],
    )
    .await?;
    drive(&state, &app, &admin, &id, "completed").await?;

    let target = backend::slack::channel_id(&team, "C1");
    let live = |ts: &'static str| {
        let state = state.clone();
        let id = backend::slack::message_id(&team, "C1", ts);
        async move {
            sql(
                &state,
                "SELECT json_build_object('n',count(*))::text FROM messages WHERE id=$1",
                &[&id],
            )
            .await
            .unwrap()[0]["n"]
                .as_i64()
        }
    };
    assert_eq!(live(d1).await, Some(0), "the completed day file is skipped");
    assert_eq!(live(d2).await, Some(1));
    assert_eq!(live(d3).await, Some(1));
    let row = sql(
        &state,
        "SELECT json_build_object('processed',processed_messages,'progress',import_progress,'channel',\
         (SELECT count(*) FROM channels WHERE id=$2))::text FROM slack_import_jobs WHERE id=$1",
        &[&id, &target],
    )
    .await?;
    assert_eq!(row[0]["processed"], 3, "1 from before + 2 new: {}", row[0]);
    assert!(
        row[0]["progress"]
            .as_str()
            .unwrap()
            .contains("2024-01-03.json"),
        "{}",
        row[0]
    );
    assert_eq!(row[0]["channel"], 1);
    Ok(())
}

/// 再取り込み (別のジョブの行) は、前のジョブの再開位置を引き継がず、全部を取り込む
#[tokio::test]
async fn test_slack_reimport_does_not_inherit_the_resume_position() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let (state, app, admin) = admin_state().await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let name = format!("reimport-{}", uuid::Uuid::new_v4().simple());
    let d1 = "1704070000.000100";
    let days = [("2024-01-01", json!([{"ts":d1,"user":"U1","text":"day 1"}]))];
    let first = importing_job(&state, &app, &admin, &team, &name, &days).await?;
    drive(&state, &app, &admin, &first, "completed").await?;
    let message = backend::slack::message_id(&team, "C1", d1);
    sql(
        &state,
        "DELETE FROM messages WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&message],
    )
    .await
    .ok();
    let target = backend::slack::channel_id(&team, "C1");
    let export = upload(
        &state,
        &app,
        &admin,
        &build_zip(&[
            (
                "users.json".into(),
                json!([{"id":"U1","team_id":team,"name":"alice"}]),
            ),
            (
                "channels.json".into(),
                json!([{"id":"C1","name":name,"members":["U1"]}]),
            ),
            (format!("{name}/2024-01-01.json"), days[0].1.clone()),
        ]),
    )
    .await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":target}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    let second = run["id"].as_str().unwrap().to_string();
    assert_ne!(first, second);
    let progress = sql(
        &state,
        "SELECT json_build_object('p',import_progress) ::text FROM slack_import_jobs WHERE id=$1",
        &[&second],
    )
    .await?;
    assert!(progress[0]["p"].is_null(), "{}", progress[0]);
    drive(&state, &app, &admin, &second, "completed").await?;
    let n = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM messages WHERE id=$1",
        &[&message],
    )
    .await?;
    assert_eq!(n[0]["n"], 1, "the re-import brings the message back");
    Ok(())
}

/// 取り込みは平文の content を messages に入れるので、暗号化チャンネルを指定した開始は 400 にする (#358)
#[tokio::test]
async fn test_slack_import_rejects_encrypted_channels() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_e2ee").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        ("channels.json".into(), json!([])),
        (
            "groups.json".into(),
            json!([{"id":"G1","name":"secret","members":["U1"]}]),
        ),
        (
            "secret/2024-01-02.json".into(),
            json!([{"ts":"1704160000.000600","user":"U1","text":"s"}]),
        ),
    ]);
    let analyzed = upload(&state, &app, &admin, &zip).await?;
    let id = analyzed["id"].as_str().unwrap().to_string();

    let mut targets = Vec::new();
    for encrypted in [true, false] {
        let channel = uuid::Uuid::new_v4().to_string();
        let (status, body) = request(
            &app,
            "POST",
            "/api/channels",
            &admin,
            Some(
                json!({"id": channel, "channel_name": format!("e2ee-{encrypted}-{suffix}"),
                "is_private": true, "encrypted": encrypted}),
            ),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        targets.push(channel);
    }

    let (status, body) = start(
        &app,
        &admin,
        &id,
        json!([{"slack_channel_id":"G1","action":"existing","channel_id":targets[0]}]),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "E2EE_PLAINTEXT_FORBIDDEN", "{body}");

    // 平文の非公開チャンネルへは今までどおり取り込める
    let (status, body) = start(
        &app,
        &admin,
        &id,
        json!([{"slack_channel_id":"G1","action":"existing","channel_id":targets[1]}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(())
}

/// 同じ名前のチャンネルの候補は、暗号化チャンネルと、画面を見ている管理者がメンバーでない非公開チャンネルの ID を返さない
#[tokio::test]
async fn test_slack_import_same_name_hides_channels_the_admin_cannot_see() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_samename").await?;
    let other = create_user(&app, "slack_other_samename").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let names: Vec<String> = ["enc", "foreign", "own", "open"]
        .iter()
        .map(|k| format!("{k}-{suffix}"))
        .collect();
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        (
            "channels.json".into(),
            json!(
                names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| json!({"id": format!("C{i}"), "name": n, "members": ["U1"]}))
                    .collect::<Vec<_>>()
            ),
        ),
        ("groups.json".into(), json!([])),
    ]);
    let analyzed = upload(&state, &app, &admin, &zip).await?;
    let id = analyzed["id"].as_str().unwrap().to_string();

    // (名前, 作る人, 非公開, 暗号化, 候補に ID が出るか)
    let cases = [
        (&names[0], &other, true, true, false),
        (&names[1], &other, true, false, false),
        (&names[2], &admin, true, false, true),
        (&names[3], &other, false, false, true),
    ];
    let mut ids = Vec::new();
    for (name, creator, private, encrypted, _) in &cases {
        let channel = uuid::Uuid::new_v4().to_string();
        let (status, body) = request(
            &app,
            "POST",
            "/api/channels",
            creator,
            Some(json!({"id": channel, "channel_name": name,
                "is_private": private, "encrypted": encrypted})),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        ids.push(channel);
    }

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let channels = body["analysis"]["channels"].as_array().unwrap();
    for (i, (name, _, _, _, visible)) in cases.iter().enumerate() {
        let entry = channels
            .iter()
            .find(|c| c["name"] == name.as_str())
            .unwrap_or_else(|| panic!("{name} is missing: {body}"));
        if *visible {
            assert_eq!(entry["same_name_channel"]["id"], ids[i], "{name}: {body}");
        } else {
            assert!(entry["same_name_channel"].is_null(), "{name}: {body}");
        }
    }
    Ok(())
}

/// ユーザーが自分で取り込む方式 (ZIP) をやめたので、管理者でない人が作ったジョブ (廃止前に作られて残っているもの、
/// 管理者でなくなった人のもの) は、権限の確認なしでは実行せず、失敗にする
#[tokio::test]
async fn test_import_job_of_a_non_admin_fails_instead_of_running() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.team_id = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin_nonadmin").await?;
    let alice = create_user(&app, "slack_alice_nonadmin").await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    sql(
        &state,
        "INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin') RETURNING json_build_object('ok',true)::text",
        &[&admin],
    )
    .await?;
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"}]),
        ),
        ("channels.json".into(), json!([])),
        (
            "groups.json".into(),
            json!([{"id":"G1","name":format!("kept-{suffix}"),"creator":"U1","members":["U1"]}]),
        ),
        (
            format!("kept-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000100","user":"U1","text":"hello"}]),
        ),
    ]);
    let export = upload(&state, &app, &admin, &zip).await?;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"G1","action":"create","name":format!("kept-{suffix}")}]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    // ジョブの作成者が管理者でなくなった (または、廃止前にユーザーが作ったジョブ)
    sql(
        &state,
        "UPDATE slack_import_jobs SET created_by=$2 WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&run, &alice],
    )
    .await?;
    let mut job = Value::Null;
    for _ in 0..100 {
        backend::slack::worker::run_once(&state).await?;
        job = request(
            &app,
            "GET",
            &format!("/api/admin/slack-imports/{run}"),
            &admin,
            None,
        )
        .await?
        .1;
        if job["status"] == "failed" || job["status"] == "completed" {
            break;
        }
    }
    assert_eq!(job["status"], "failed", "{job}");
    assert!(
        job["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Only workspace admins"),
        "{job}"
    );
    let created = sql(
        &state,
        "SELECT json_build_object('id',id)::text FROM channels WHERE id=$1",
        &[&backend::slack::channel_id(&team, "G1")],
    )
    .await?;
    assert!(created.is_empty(), "{created:?}");
    // ユーザー向けの取り込み API は、もうない
    let (status, _) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}
