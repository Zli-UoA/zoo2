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
    let export = upload(&state, &app, &admin, &replies_only).await;
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
    let analyzed = upload(&state, &app, &admin, &turned_private).await;
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
    .await;
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
    .await;
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
    let analyzed = upload(&state, &app, &admin, &later).await;
    let (status, body) = start(
        &app,
        &admin,
        analyzed["id"].as_str().unwrap(),
        json!([
            {"slack_channel_id":"C1","action":"skip"},
            {"slack_channel_id":"C2","action":"create","name":format!("random-{suffix}")}
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
    let analyzed = upload(&state, &app, &admin, &quiet).await;
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
        let (status, body) = start(&app, &admin, &export_id, mappings).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        drive(
            &state,
            &app,
            &admin,
            body["id"].as_str().unwrap(),
            "completed",
        )
        .await;
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
    let exists = || async {
        state
            .s3_client
            .head_object()
            .bucket(&config.s3.bucket)
            .key(&key)
            .send()
            .await
            .is_ok()
    };
    assert!(exists().await);

    // 署名付き URL の期限内は、まだ PUT が続いているかもしれないので消さない
    backend::slack::worker::cleanup(&state).await?;
    assert!(exists().await);
    // 期限と猶予を過ぎたら、もう一度だけ消す
    sql(
        &state,
        "UPDATE slack_import_jobs SET updated_at=now()-interval '30 days' WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&id],
    )
    .await?;
    backend::slack::worker::cleanup(&state).await?;
    assert!(!exists().await, "the late upload must be removed");
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
    let export = upload(&state, &app, &admin, &zip).await;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("files-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await;
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
    let export = upload(&no_token, &app, &admin, &zip).await;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("refetch-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    drive(&no_token, &app, &admin, &run, "completed").await;
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
    drive(&small_state, &app_small, &admin, &run, "completed").await;
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
    drive(&big_state, &app_big, &admin, &run, "completed").await;
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
    let export = upload(&state, &app, &admin, &zip).await;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("samename-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    drive(&state, &app, &admin, &run, "completed").await;
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
    drive(&small_state, &app_small, &admin, &run, "completed").await;
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
    let export = upload(&state, &app, &admin, &zip).await;
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
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await;
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
    .await;
    assert_eq!(status, StatusCode::OK, "{rerun}");
    drive(
        &state,
        &app,
        &admin,
        rerun["id"].as_str().unwrap(),
        "completed",
    )
    .await;
    let after = stamp(format!("flag-keep-{suffix}")).await;
    assert_ne!(
        before[0]["at"], after[0]["at"],
        "the import time is updated"
    );
    let again = row(format!("flag-keep-{suffix}")).await;
    assert_eq!(again[0]["kind"], "created", "the first kind is kept");
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
    let cases = [
        ("completed", "create", "created"),
        ("failed", "create", "created"),
        ("cancelled", "create", "created"),
        ("completed", "existing", "existing"),
    ];
    let mut channel_ids = Vec::new();
    for (index, (status, action, _)) in cases.iter().enumerate() {
        let channel_id = uuid::Uuid::new_v4().to_string();
        let slack_channel = format!("C{index}");
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
    for ((status, action, expected), channel_id) in cases.iter().zip(&channel_ids) {
        let rows = query(
            &mut tx,
            "SELECT json_build_object('kind',slack_import_kind,'at',slack_imported_at IS NOT NULL)::text \
             FROM channels WHERE id=$1",
            &[channel_id],
        )
        .await?;
        assert_eq!(rows[0]["kind"], *expected, "{action} job in {status}");
        assert_eq!(rows[0]["at"], true);
    }
    // コミットしない (ロールバックされる)
    drop(tx);
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
    let export = upload(&state, &app, &admin, &zip).await;
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C1","action":"create","name":format!("redir-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await;
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
    let export = upload(&state, &app, &admin, &zip).await;
    let suggested = export["analysis"]["channels"][0]["suggested_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert_ne!(suggested, format!("slack-general-{suffix}"), "{export}");
    assert!(suggested.starts_with("slack-general-"), "{suggested}");
    assert!(suggested.chars().count() <= 80, "{suggested}");
    Ok(())
}

#[tokio::test]
async fn test_linked_users_import_their_own_channels() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    // ワークスペースは毎回作るので、.env の SLACK_TEAM_ID に左右されないようにする
    config.slack.team_id = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let admin = create_user(&app, "slack_admin5").await?;
    let alice = create_user(&app, "slack_alice").await?;
    let bob = create_user(&app, "slack_bob").await?;
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        // alice は Slack の U1 として連携済み
        toasty::sql::statement(
            "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U1','x')",
        )
        .bind(alice.as_str())
        .bind(team.as_str())
        .exec(&mut tx)
        .await?;
        toasty::sql::statement(
            "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked) VALUES($1,'U1',$2,true)",
        )
        .bind(team.as_str())
        .bind(alice.as_str())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    // 連携していなければ、取り込めるものはない
    let (status, none) = request(&app, "GET", "/api/slack/imports", &bob, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(none["export"], Value::Null);

    let shared = format!("shared-{suffix}");
    let zip = build_zip(&[
        (
            "users.json".into(),
            json!([{"id":"U1","team_id":team,"name":"alice"},{"id":"U2","team_id":team,"name":"bob"}]),
        ),
        (
            "channels.json".into(),
            json!([
                {"id":"C1","name":shared,"creator":"U1","members":["U1","U2"]},
                {"id":"C2","name":format!("bobs-{suffix}"),"creator":"U2","members":["U2"]}
            ]),
        ),
        (
            "groups.json".into(),
            json!([
                {"id":"G1","name":format!("secret-{suffix}"),"creator":"U1","members":["U1"]},
                {"id":"G2","name":format!("old-{suffix}"),"creator":"U1","members":["U1"],"is_archived":true}
            ]),
        ),
        (
            format!("old-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000400","user":"U1","text":"archived"}]),
        ),
        (
            format!("{shared}/2024-01-01.json"),
            json!([{"ts":"1704070000.000100","user":"U1","text":"mine"}]),
        ),
        (
            format!("bobs-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000200","user":"U2","text":"bob"}]),
        ),
        (
            format!("secret-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000300","user":"U1","text":"secret"}]),
        ),
    ]);
    // 管理者が ZIP を保存するだけで、取り込みは各ユーザーが行う
    let export = upload(&state, &app, &admin, &zip).await;

    // Slack と同じ名前の既存チャンネル (alice はまだ参加していない)
    let same = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &app,
        "POST",
        "/api/channels",
        &admin,
        Some(json!({"id": same, "channel_name": shared, "is_private": false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let (status, own) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["export"]["id"], export["id"]);
    let ids: Vec<&str> = own["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["slack_channel_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["C1", "G1", "G2"],
        "only the channels alice created"
    );
    assert_eq!(
        own["channels"][0]["targets"],
        json!([]),
        "alice has not joined it"
    );

    // SLACK_TEAM_ID を後から別のワークスペースにすると、残っている連携では取り込めない
    let mut restricted = state.clone();
    restricted.config.slack.team_id = Some(format!("T{}", uuid::Uuid::new_v4().simple()));
    let restricted = create_router(restricted);
    let (status, hidden) = request(&restricted, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hidden["export"], Value::Null, "{hidden}");
    assert_eq!(hidden["channels"], json!([]));
    let (status, _) = request(
        &restricted,
        "POST",
        "/api/slack/imports",
        &alice,
        Some(json!({"mappings": [{"slack_channel_id":"C1","action":"create","name":format!("blocked-{suffix}")}]})),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // 同じワークスペースを許可していれば、そのまま取り込める
    let mut allowed = state.clone();
    allowed.config.slack.team_id = Some(team.clone());
    let (status, same_team) = request(
        &create_router(allowed),
        "GET",
        "/api/slack/imports",
        &alice,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(same_team["export"]["id"], export["id"]);

    let import = |mappings: Value| {
        let app = app.clone();
        let alice = alice.clone();
        async move {
            request(
                &app,
                "POST",
                "/api/slack/imports",
                &alice,
                Some(json!({ "mappings": mappings })),
            )
            .await
            .unwrap()
        }
    };
    // 他人が作ったチャンネルや、参加していない同名チャンネルには取り込めない
    let (status, _) = import(
        json!([{"slack_channel_id":"C2","action":"create","name":format!("taken-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) =
        import(json!([{"slack_channel_id":"C1","action":"existing","channel_id":same}])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP)",
        )
        .bind(alice.as_str())
        .bind(same.as_str())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }
    let (_, own) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(own["channels"][0]["targets"][0]["id"], same.as_str());
    // 非公開チャンネルは、公開を指定しても非公開で作る
    let (status, run) = import(json!([
        {"slack_channel_id":"C1","action":"existing","channel_id":same},
        {"slack_channel_id":"G1","action":"create","name":format!("secret-{suffix}"),"is_private":false}
    ]))
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["created_by"], alice.as_str());
    assert_eq!(run["total_messages"], 2);
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await;

    let rows = sql(
        &state,
        "SELECT json_build_object('channel',channel_id,'user',user_id)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(&team, "C1", "1704070000.000100")],
    )
    .await?;
    assert_eq!(rows[0], json!({"channel": same, "user": alice}));
    let secret = sql(
        &state,
        "SELECT json_build_object('private',is_private,'creator',created_user_id)::text FROM channels WHERE id=$1",
        &[&backend::slack::channel_id(&team, "G1")],
    )
    .await?;
    assert_eq!(secret[0], json!({"private": true, "creator": alice}));
    let (_, own) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(own["imports"][0]["status"], "completed");
    assert_eq!(
        own["channels"][1]["existing_channel"]["id"],
        backend::slack::channel_id(&team, "G1")
    );
    let g1 = backend::slack::channel_id(&team, "G1");
    assert_eq!(own["channels"][1]["targets"][0]["id"], g1.as_str());

    // 投稿を管理者に限ったチャンネルには、通常の投稿と同じく一般メンバーは取り込めない
    sql(
        &state,
        "UPDATE channels SET posting_policy='managers' WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&same],
    )
    .await?;
    let (_, own) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(own["channels"][0]["targets"], json!([]));
    let (status, _) =
        import(json!([{"slack_channel_id":"C1","action":"existing","channel_id":same}])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    sql(
        &state,
        "UPDATE channels SET posting_policy='members' WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&same],
    )
    .await?;

    // 取り込みは ZIP のメンバーも足すので、取り込み先は現在の参加状況とメンバー追加の権限で絞る
    let targets = || async {
        let (_, own) = request(&app, "GET", "/api/slack/imports", &alice, None)
            .await
            .unwrap();
        own["channels"][1]["targets"].clone()
    };
    let exec = |statement: &'static str| {
        let db = db.clone();
        let (alice, admin, g1) = (alice.clone(), admin.clone(), g1.clone());
        async move {
            let mut db = db;
            let mut tx = db.transaction().await.unwrap();
            toasty::sql::statement(statement)
                .bind(g1.as_str())
                .bind(alice.as_str())
                .bind(admin.as_str())
                .exec(&mut tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
    };
    // 非公開チャンネルの一般メンバーは、メンバーを追加できない
    exec("UPDATE channels SET created_user_id=$3 WHERE id=$1 AND $2<>$3").await;
    assert_eq!(targets().await, json!([]));
    let (status, _) =
        import(json!([{"slack_channel_id":"G1","action":"existing","channel_id":g1}])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // 以前の取り込み先から外されたら、取り込み直して戻ることはできない
    exec("UPDATE channels SET created_user_id=$2 WHERE id=$1 AND $2<>$3").await;
    assert_eq!(targets().await[0]["id"], g1.as_str());
    // 開始した後で外された場合も、ワーカーが取り込みで戻さない
    let (status, run) =
        import(json!([{"slack_channel_id":"G1","action":"existing","channel_id":g1}])).await;
    assert_eq!(status, StatusCode::OK, "{run}");
    let run = run["id"].as_str().unwrap().to_string();
    exec("DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2 AND $2<>$3").await;
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
            .contains("no longer"),
        "{job}"
    );
    let rejoined = sql(
        &state,
        "SELECT json_build_object('user_id',user_id)::text FROM channel_user WHERE channel_id=$1 AND user_id=$2",
        &[&g1, &alice],
    )
    .await?;
    assert!(rejoined.is_empty(), "{rejoined:?}");
    // 参加していない非公開チャンネルの名前と ID は、取り込み先の候補としても返さない
    let (_, own) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(own["channels"][1]["slack_channel_id"], "G1");
    assert_eq!(own["channels"][1]["existing_channel"], Value::Null, "{own}");
    assert_eq!(own["channels"][1]["existing_hidden"], true, "{own}");
    assert!(!own["channels"].to_string().contains(&g1), "{own}");
    assert_eq!(
        own["channels"][1]["same_name_channel"],
        Value::Null,
        "{own}"
    );
    assert_eq!(own["channels"][1]["targets"], json!([]));
    assert_eq!(targets().await, json!([]));
    let (status, _) =
        import(json!([{"slack_channel_id":"G1","action":"existing","channel_id":g1}])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // 作り直しを指定しても、以前作ったチャンネルへは戻れない
    let (status, _) = import(
        json!([{"slack_channel_id":"G1","action":"create","name":format!("secret-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Slack でアーカイブ済みのチャンネルは、取り込みの最後に取り込み先もアーカイブする
    // (途中でアーカイブすると、再開したときに取り込み先の確認で止まるため)
    let (status, run) =
        import(json!([{"slack_channel_id":"G2","action":"create","name":format!("old-{suffix}")}]))
            .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    drive(
        &state,
        &app,
        &admin,
        run["id"].as_str().unwrap(),
        "completed",
    )
    .await;
    let old = sql(
        &state,
        "SELECT json_build_object('archived',c.archive_at IS NOT NULL,\
         'messages',(SELECT count(*) FROM messages m WHERE m.channel_id=c.id))::text FROM channels c WHERE c.id=$1",
        &[&backend::slack::channel_id(&team, "G2")],
    )
    .await?;
    assert_eq!(old[0], json!({"archived": true, "messages": 1}));

    // 見えない取り込み先 (参加していない非公開チャンネル) がある Slack チャンネルを「新しいチャンネル」で
    // 取り込もうとしても、取り込み先の名前を含むエラーは返さない
    sql(
        &state,
        "UPDATE channels SET is_private=true WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&same],
    )
    .await?;
    sql(
        &state,
        "DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2 RETURNING json_build_object('ok',true)::text",
        &[&same, &alice],
    )
    .await?;
    let (status, body) = import(
        json!([{"slack_channel_id":"C1","action":"create","name":format!("hidden-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!body.to_string().contains("imported into #"), "{body}");
    sql(
        &state,
        "UPDATE channels SET is_private=false WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&same],
    )
    .await?;

    // ゲストは、通常の API と同じくチャンネルを作れない
    exec("UPDATE users SET is_guest=true WHERE id=$2 AND $1<>$3").await;
    let (status, body) = import(
        json!([{"slack_channel_id":"C1","action":"create","name":format!("guest-{suffix}")}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.to_string().contains("Guests"), "{body}");

    // ゲストには、参加していない公開チャンネルの名前と ID も見せない (チャンネル一覧と同じ)
    sql(
        &state,
        "DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2 RETURNING json_build_object('ok',true)::text",
        &[&same, &alice],
    )
    .await?;
    let (_, own) = request(&app, "GET", "/api/slack/imports", &alice, None).await?;
    assert_eq!(own["channels"][0]["slack_channel_id"], "C1");
    assert_eq!(own["channels"][0]["existing_channel"], Value::Null, "{own}");
    assert_eq!(
        own["channels"][0]["same_name_channel"],
        Value::Null,
        "{own}"
    );
    assert_eq!(own["channels"][0]["existing_hidden"], true, "{own}");
    assert!(!own["channels"].to_string().contains(&same), "{own}");
    Ok(())
}

/// 取り込みを始めた後でゲストになった場合も、ワーカーは新しいチャンネルを作らない
#[tokio::test]
async fn test_self_import_does_not_create_channels_after_becoming_guest() -> anyhow::Result<()> {
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
    let admin = create_user(&app, "slack_admin_guest").await?;
    let alice = create_user(&app, "slack_alice_guest").await?;
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
            json!([{"id":"C9","name":format!("later-{suffix}"),"creator":"U1","members":["U1"]}]),
        ),
        (
            format!("later-{suffix}/2024-01-01.json"),
            json!([{"ts":"1704070000.000100","user":"U1","text":"mine"}]),
        ),
    ]);
    upload(&state, &app, &admin, &zip).await;
    let (status, run) = request(
        &app,
        "POST",
        "/api/slack/imports",
        &alice,
        Some(json!({"mappings":[{"slack_channel_id":"C9","action":"create","name":format!("later-{suffix}")}]})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{run}");
    // 開始した後でゲストになった
    sql(
        &state,
        "UPDATE users SET guest_kind='multi' WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&alice],
    )
    .await?;
    let job = drive(&state, &app, &admin, run["id"].as_str().unwrap(), "failed").await;
    assert!(
        job["error"].as_str().unwrap_or_default().contains("Guests"),
        "{job}"
    );
    let created = sql(
        &state,
        "SELECT json_build_object('id',id)::text FROM channels WHERE id=$1",
        &[&backend::slack::channel_id(&team, "C9")],
    )
    .await?;
    assert!(created.is_empty(), "{created:?}");
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
    let export = upload(&state, &app, &admin, &zip).await;
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

    // 利用者の「新しいチャンネル」は、前回の取り込み先として記録されていないチャンネルを再利用しない
    let (status, body) = request(
        &app,
        "POST",
        "/api/slack/imports",
        &alice,
        Some(json!({"mappings":[{"slack_channel_id":"C7","action":"create","name":format!("mine-{suffix}")}]})),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!body.to_string().contains("secret-"), "{body}");

    // 管理者の取り込みでも、ワーカーが同じ確認をし、失敗理由に名前を出さない
    let (status, run) = start(
        &app,
        &admin,
        export["id"].as_str().unwrap(),
        json!([{"slack_channel_id":"C7","action":"create","name":format!("mine-{suffix}")}]),
    )
    .await;
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
