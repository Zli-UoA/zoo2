use std::io::{Cursor, Write};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
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
async fn drive(state: &AppState, app: &Router, admin: &str, id: &str, until: &str) -> Value {
    for _ in 0..200 {
        let (_, job) = request(
            app,
            "GET",
            &format!("/api/admin/slack-imports/{id}"),
            admin,
            None,
        )
        .await
        .unwrap();
        if job["status"] == until {
            return job;
        }
        assert_ne!(job["status"], "failed", "import failed: {job}");
        if !backend::slack::worker::run_once(state).await.unwrap() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    panic!("import {id} did not reach {until}");
}

/// ZIP を作ってアップロードし、解析まで進める
async fn upload(state: &AppState, app: &Router, admin: &str, zip: &[u8]) -> Value {
    let (status, created) = request(
        app,
        "POST",
        "/api/admin/slack-imports",
        admin,
        Some(json!({"file_name": "export.zip", "file_size": zip.len()})),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::OK, "{created}");
    let put = reqwest::Client::new()
        .put(created["upload_url"].as_str().unwrap())
        .header("content-type", "application/zip")
        .body(zip.to_vec())
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "{:?}", put.text().await);
    let id = created["import"]["id"].as_str().unwrap().to_string();
    let (status, job) = request(
        app,
        "POST",
        &format!("/api/admin/slack-imports/{id}/uploaded"),
        admin,
        None,
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::OK, "{job}");
    drive(state, app, admin, &id, "ready").await
}

/// 保存したエクスポートから取り込みを始め、取り込みのジョブを返す
async fn start(app: &Router, admin: &str, export: &str, mappings: Value) -> (StatusCode, Value) {
    request(
        app,
        "POST",
        &format!("/api/admin/slack-imports/{export}/start"),
        admin,
        Some(json!({ "mappings": mappings })),
    )
    .await
    .unwrap()
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
                {"ts":"1704070100.000200","user":"U2","text":"reply &amp; more","thread_ts":"1704070000.000100"},
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

    let job = upload(&state, &app, &admin, &zip).await;
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
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["source_id"], id.as_str());
    let run = started["id"].as_str().unwrap().to_string();
    let done = drive(&state, &app, &admin, &run, "completed").await;
    assert_eq!(done["processed_messages"], 6);
    assert_eq!(done["total_files"], 3);
    assert_eq!(done["processed_files"], 3);
    assert_eq!(done["pending_files"], 0);
    // F3 は取得に失敗したまま完了し、管理者が取得し直せる
    assert_eq!(done["failed_files"], 1, "{done}");
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/admin/slack-imports/{run}/fetch-files"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let done = drive(&state, &app, &admin, &run, "completed").await;
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
    assert_eq!(messages[1]["content"], "reply & more");
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
    let stored = state
        .s3_client
        .get_object()
        .bucket(&config.s3.bucket)
        .key(attachments[0]["key"].as_str().unwrap())
        .send()
        .await?
        .body
        .collect()
        .await?
        .into_bytes();
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
    let zip_exists = |key: String| {
        let client = state.s3_client.clone();
        let bucket = config.s3.bucket.clone();
        async move {
            client
                .head_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
                .is_ok()
        }
    };
    assert!(zip_exists(format!("slack-imports/{id}/export.zip")).await);
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
    let again = upload(&state, &app, &admin, &zip).await;
    let (_, old) = request(
        &app,
        "GET",
        &format!("/api/admin/slack-imports/{id}"),
        &admin,
        None,
    )
    .await?;
    assert_eq!(old["status"], "cancelled");
    assert!(!zip_exists(format!("slack-imports/{id}/export.zip")).await);
    let (status, _) = start(
        &app,
        &admin,
        &id,
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":general}]),
    )
    .await;
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
    .await;
    assert_eq!(status, StatusCode::OK);
    let again_id = again_run["id"].as_str().unwrap().to_string();
    let again_done = drive(&state, &app, &admin, &again_id, "completed").await;
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
    let third = upload(&state, &app, &admin, &zip).await;
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
                 ]}
            ]),
        ),
        ("general/2024-01-02.json".into(), json!([reply])),
    ]);

    // 同じ Slack チャンネルを別々の取り込み先へ入れるジョブを、どちらも開始してしまう
    let export = upload(&state, &app, &admin, &replies_only).await;
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
        let (status, body) = start(&app, &admin, &export_id, json!([mapping])).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        runs.push(body["id"].as_str().unwrap().to_string());
    }
    let (first_id, second_id) = (runs[0].clone(), runs[1].clone());
    // 先に処理されたほうだけが成功し、もう一方は取り込み先が食い違うので失敗する
    drive(&state, &app, &admin, &first_id, "completed").await;
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
    let again = upload(&state, &app, &admin, &both).await;
    let (status, body) = start(
        &app,
        &admin,
        again["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"existing","channel_id":general}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(
        &state,
        &app,
        &admin,
        body["id"].as_str().unwrap(),
        "completed",
    )
    .await;
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
    let analyzed = upload(&state, &app, &admin, &first).await;
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
    .await;
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
    let done = drive(&state, &app, &admin, &run, "completed").await;
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
    let again = upload(&state, &app, &admin, &parent_only).await;
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
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    drive(
        &state,
        &app,
        &admin,
        body["id"].as_str().unwrap(),
        "completed",
    )
    .await;
    let rows = sql(
        &state,
        "SELECT json_build_object('parent_id',parent_id)::text FROM messages WHERE id=$1",
        &[&message(reply_ts)],
    )
    .await?;
    assert_eq!(rows[0]["parent_id"], json!(message(parent_ts)));
    Ok(())
}
