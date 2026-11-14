//! Slack の添付の行単位の取得。ジョブに紐づかない (差分同期の) 添付と、終わったジョブの添付を、
//! `orphan_files_once` が 1 件ずつ取得することを、モックのファイルサーバーに対して確かめる。
//! テストごとに一意なワークスペース (`team`) を使い、`orphan_files_once` もそのワークスペースだけを取るので、
//! 同じバイナリの中のほかのテストと並列に動かしても、互いの行を取らない。
//! ただし、ジョブの取得 (`run_once`) は DB 上のどのジョブでも拾うので、それを使うテストは `SERIAL` で順に実行する。

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
    apps::query, config::Config, db::init_db, routes::create_router, slack::worker,
    state::AppState, storage::ensure_bucket_exists,
};

async fn request(app: &Router, uri: &str, user: &str, body: Value) -> anyhow::Result<StatusCode> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("X-Dev-User", user)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body)?))?,
        )
        .await?;
    let status = response.status();
    let _ = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(status)
}

async fn sql(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(rows)
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Fixture {
    state: AppState,
    team: String,
    user: String,
    channel: String,
}

/// モックのファイルサーバー (`/files/…`) を許可 URL にした状態と、ユーザー・チャンネルを作る
async fn fixture(files: &MockServer, mut tune: impl FnMut(&mut Config)) -> anyhow::Result<Fixture> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.import_bot_token = Some("xoxb-test".into());
    config.slack.files_base_url = format!("{}/files/", files.uri());
    tune(&mut config);
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config.clone());
    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    let app = create_router(state.clone());
    let user = uuid::Uuid::new_v4().to_string();
    let status = request(
        &app,
        "/api/users",
        &user,
        json!({"id": user, "user_name": format!("files_{}", uuid::Uuid::new_v4().simple()), "display_name": "files"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let channel = uuid::Uuid::new_v4().to_string();
    let status = request(
        &app,
        "/api/channels",
        &user,
        json!({"id": channel, "channel_name": format!("files_{}", uuid::Uuid::new_v4().simple()), "is_private": false}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(Fixture {
        state,
        team: format!("TFILES{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
        user,
        channel,
    })
}

struct Row<'a> {
    file: &'a str,
    job: &'a str,
    content: &'a str,
    in_content: bool,
    size: i64,
}

impl Fixture {
    /// 投稿と `slack_files` の行を作る。`url` は `/files/<file>/<file>.txt`。投稿の ID を返す
    async fn add(&self, row: Row<'_>) -> anyhow::Result<String> {
        let url = format!(
            "{}{}/{}.txt",
            self.state.config.slack.files_base_url, row.file, row.file
        );
        self.add_url(row, &url).await
    }

    async fn add_url(&self, row: Row<'_>, url: &str) -> anyhow::Result<String> {
        let message = uuid::Uuid::new_v4().to_string();
        sql(
            &self.state,
            "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,$4,'2026-01-01 00:00:00') \
             RETURNING json_build_object('ok',true)::text",
            &[&message, &self.user, &self.channel, row.content],
        )
        .await?;
        sql(
            &self.state,
            "INSERT INTO slack_files(team_id,slack_file_id,job_id,message_id,channel_id,uploader_id,name,mimetype,size,\
             url_private_download,in_content) VALUES($1,$2,NULLIF($3,''),$4,$5,$6,$7,'text/plain',$8::bigint,$9,$10::boolean) \
             RETURNING json_build_object('ok',true)::text",
            &[
                &self.team,
                row.file,
                row.job,
                &message,
                &self.channel,
                &self.user,
                &format!("{}.txt", row.file),
                &row.size.to_string(),
                url,
                if row.in_content { "true" } else { "false" },
            ],
        )
        .await?;
        Ok(message)
    }

    async fn job(&self, status: &str) -> anyhow::Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        sql(
            &self.state,
            // ジョブの取得 (`run_once`) が、このテストのジョブを勝手に拾わないよう、次に動く時刻は先にしておく
            "INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id,available_at) \
             VALUES($1,$2,$3,'x.zip',1,'slack-imports/x.zip',$4,now()+interval '1 day') RETURNING json_build_object('ok',true)::text",
            &[&id, &self.user, status, &self.team],
        )
        .await?;
        Ok(id)
    }

    async fn file(&self, file: &str) -> Value {
        sql(
            &self.state,
            "SELECT json_build_object('status',status,'attempts',attempts,'error',error,'attachment',attachment_id,\
             'in_content',in_content,'leased',lease_token IS NOT NULL,'waiting',lease_until>now(),'job',job_id)::text \
             FROM slack_files WHERE team_id=$1 AND slack_file_id=$2",
            &[&self.team, file],
        )
        .await
        .unwrap()
        .remove(0)
    }

    async fn content(&self, message: &str) -> String {
        sql(
            &self.state,
            "SELECT json_build_object('c',content)::text FROM messages WHERE id=$1",
            &[message],
        )
        .await
        .unwrap()[0]["c"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn once(&self) -> bool {
        worker::orphan_files_once(&self.state, &self.team)
            .await
            .unwrap()
    }

    /// 失敗後の待ちを終わらせる
    async fn expire_wait(&self) {
        sql(
            &self.state,
            "UPDATE slack_files SET lease_until=now()-interval '1 second' WHERE team_id=$1 \
             RETURNING json_build_object('ok',true)::text",
            &[&self.team],
        )
        .await
        .unwrap();
    }
}

async fn serve(files: &MockServer, file: &str, body: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/files/{file}/{file}.txt")))
        .and(header("authorization", "Bearer xoxb-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_bytes(body.to_vec()),
        )
        .mount(files)
        .await;
}

#[tokio::test]
async fn test_orphan_files_fetch_sync_rows_and_remove_the_filled_name() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"hello").await;
    let f = fixture(&files, |_| {}).await?;
    // 差分同期の形: ジョブなし、本文は同期が補ったファイル名
    let message = f
        .add(Row {
            file: "F1",
            job: "",
            content: "F1.txt",
            in_content: true,
            size: 5,
        })
        .await?;

    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "done");
    assert_eq!(row["in_content"], false);
    assert_eq!(row["leased"], false);
    let attachment = row["attachment"].as_str().unwrap().to_string();
    let found = sql(
        &f.state,
        "SELECT json_build_object('name',file_name,'uploader',uploader_id,'message',message_id)::text \
         FROM attachments WHERE id=$1",
        &[&attachment],
    )
    .await?;
    assert_eq!(found[0]["name"], "F1.txt");
    assert_eq!(found[0]["message"], message.as_str());
    // 補ったファイル名は、添付と二重に出ないよう本文から外れる
    assert_eq!(f.content(&message).await, "");
    // もう取る行はない
    assert!(!f.once().await);
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_take_rows_of_finished_jobs_but_not_active_ones() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    for name in ["F1", "F2", "F3", "F4", "F5"] {
        serve(&files, name, b"x").await;
    }
    let f = fixture(&files, |_| {}).await?;
    let cancelled = f.job("cancelled").await?;
    let completed = f.job("completed").await?;
    let failed = f.job("failed").await?;
    let importing = f.job("importing").await?;
    let ready = f.job("ready").await?;
    for (file, job) in [
        ("F1", &cancelled),
        ("F2", &completed),
        ("F3", &failed),
        ("F4", &importing),
        ("F5", &ready),
    ] {
        f.add(Row {
            file,
            job,
            content: "",
            in_content: false,
            size: 1,
        })
        .await?;
    }

    // 終わったジョブの 3 件だけが取れて、進行中・開始前のジョブの行は、ジョブの取得に任せる
    for _ in 0..3 {
        assert!(f.once().await);
    }
    assert!(!f.once().await);
    for file in ["F1", "F2", "F3"] {
        assert_eq!(f.file(file).await["status"], "done", "{file}");
    }
    for file in ["F4", "F5"] {
        assert_eq!(f.file(file).await["status"], "pending", "{file}");
    }
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_retry_with_a_wait_and_give_up_after_three_failures() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/F1/F1.txt"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&files)
        .await;
    let f = fixture(&files, |_| {}).await?;
    // 本文のファイル名は、取得に失敗しても残る
    let message = f
        .add(Row {
            file: "F1",
            job: "",
            content: "F1.txt",
            in_content: true,
            size: 1,
        })
        .await?;

    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(
        (row["status"].as_str(), row["attempts"].as_i64()),
        (Some("pending"), Some(1))
    );
    assert_eq!(
        (row["leased"].as_bool(), row["waiting"].as_bool()),
        (Some(false), Some(true))
    );
    // 待ちの間は取らない (3 回を短い間に使い切らない)
    assert!(!f.once().await);

    f.expire_wait().await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["attempts"], 2);
    f.expire_wait().await;
    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "failed");
    assert_eq!(row["attempts"], 3);
    assert!(row["error"].as_str().unwrap().contains("500"));
    assert_eq!(f.content(&message).await, "F1.txt");
    assert!(!f.once().await);
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_do_not_count_a_429_as_an_attempt() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/F1/F1.txt"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&files)
        .await;
    serve(&files, "F1", b"ok").await;
    let f = fixture(&files, |_| {}).await?;
    f.add(Row {
        file: "F1",
        job: "",
        content: "",
        in_content: false,
        size: 2,
    })
    .await?;

    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "pending");
    assert_eq!(row["attempts"], 0);
    assert_eq!(row["waiting"], true);
    assert!(!f.once().await);
    f.expire_wait().await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_skip_oversized_files_and_keep_the_name() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"0123456789").await;
    let f = fixture(&files, |config| config.s3.max_file_size = 5).await?;
    // 記録上のサイズは小さいが、実際は上限を超えている。取得してからスキップする
    let message = f
        .add(Row {
            file: "F1",
            job: "",
            content: "",
            in_content: false,
            size: 1,
        })
        .await?;
    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "skipped");
    assert!(row["error"].as_str().unwrap().contains("exceeds"));
    // 添付だけの投稿が空にならないよう、名前を本文に足す
    assert_eq!(f.content(&message).await, "F1.txt");
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_give_up_rows_that_keep_stopping_the_worker() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, |_| {}).await?;
    f.add(Row {
        file: "F1",
        job: "",
        content: "",
        in_content: false,
        size: 1,
    })
    .await?;
    // 取得の途中でプロセスが落ちて、リースが切れた形 (3 回取ったあと)
    sql(
        &f.state,
        "UPDATE slack_files SET lease_token='dead',lease_until=now()-interval '1 minute',attempts=3 WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    assert!(!f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "failed");
    assert_eq!(row["leased"], false);
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_retake_an_expired_lease() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"x").await;
    let f = fixture(&files, |_| {}).await?;
    f.add(Row {
        file: "F1",
        job: "",
        content: "",
        in_content: false,
        size: 1,
    })
    .await?;
    // 別の台が取っている間 (リースが有効) は取らない。切れたら取り直す
    sql(
        &f.state,
        "UPDATE slack_files SET lease_token='other',lease_until=now()+interval '5 minutes',attempts=1 WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    assert!(!f.once().await);
    f.expire_wait().await;
    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(
        (row["status"].as_str(), row["attempts"].as_i64()),
        (Some("done"), Some(2))
    );
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_ignore_deleted_channels() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"x").await;
    let f = fixture(&files, |_| {}).await?;
    f.add(Row {
        file: "F1",
        job: "",
        content: "",
        in_content: false,
        size: 1,
    })
    .await?;
    sql(
        &f.state,
        "UPDATE channels SET deleted_at=CURRENT_TIMESTAMP WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.channel],
    )
    .await?;
    assert!(!f.once().await);
    assert_eq!(f.file("F1").await["status"], "pending");
    Ok(())
}

/// 管理画面の「添付ファイルを取得」(ジョブの取得) は、行単位の取得が取っている行 (リースが有効) を飛ばす。
/// 行単位の取得が書く前に、ジョブの取得が同じファイルを取得して失敗させることがない。リースが切れたら取る
#[tokio::test]
async fn test_job_fetch_skips_rows_leased_by_the_file_worker() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"x").await;
    let f = fixture(&files, |_| {}).await?;
    let job = f.job("completed").await?;
    let message = f
        .add(Row {
            file: "F1",
            job: &job,
            content: "F1.txt",
            in_content: true,
            size: 1,
        })
        .await?;
    // 行単位の取得が取っている間 (リースが有効)
    sql(
        &f.state,
        "UPDATE slack_files SET lease_token='row',lease_until=now()+interval '5 minutes',attempts=1 WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    // 管理者が再取得を始めた形 (ジョブを取得の段階に戻す)
    sql(
        &f.state,
        "UPDATE slack_import_jobs SET status='fetching_files',available_at=now(),finished_at=NULL WHERE id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&job],
    )
    .await?;
    while worker::run_once(&f.state).await? {}
    let row = f.file("F1").await;
    assert_eq!(row["status"], "pending");
    assert_eq!(row["leased"], true);
    assert_eq!(files.received_requests().await.unwrap().len(), 0);
    assert_eq!(f.content(&message).await, "F1.txt");

    // 持ち主が落ちて、リースが切れたら、ジョブの取得が取得する (リースは結果を書くときに外れる)
    f.expire_wait().await;
    sql(
        &f.state,
        "UPDATE slack_import_jobs SET status='fetching_files',available_at=now(),finished_at=NULL WHERE id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&job],
    )
    .await?;
    while worker::run_once(&f.state).await? {}
    let row = f.file("F1").await;
    assert_eq!(row["status"], "done");
    assert_eq!(row["leased"], false);
    assert_eq!(f.content(&message).await, "");
    Ok(())
}

/// 行単位の取得が、終わったジョブの添付を取得したら、そのジョブの `processed_files` も合う
#[tokio::test]
async fn test_orphan_files_update_processed_files_of_finished_jobs() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"x").await;
    let f = fixture(&files, |_| {}).await?;
    let job = f.job("completed").await?;
    sql(
        &f.state,
        "UPDATE slack_import_jobs SET total_files=1,processed_files=0 WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&job],
    )
    .await?;
    f.add(Row {
        file: "F1",
        job: &job,
        content: "",
        in_content: false,
        size: 1,
    })
    .await?;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    let jobs = sql(
        &f.state,
        "SELECT json_build_object('n',processed_files)::text FROM slack_import_jobs WHERE id=$1",
        &[&job],
    )
    .await?;
    assert_eq!(jobs[0]["n"], 1);
    Ok(())
}

/// 投稿者が、補われたファイル名の本文を書き換えたら、添付を取得できても、その本文は外さない
#[tokio::test]
async fn test_orphan_files_keep_the_text_the_author_edited() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    serve(&files, "F1", b"x").await;
    let f = fixture(&files, |_| {}).await?;
    let message = f
        .add(Row {
            file: "F1",
            job: "",
            content: "F1.txt",
            in_content: true,
            size: 1,
        })
        .await?;
    let app = create_router(f.state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/messages/{message}"))
                .header("X-Dev-User", &f.user)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(
                    &json!({"content": "F1.txt"}),
                )?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(f.file("F1").await["in_content"], false);
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    // 投稿者が書いた本文は、添付と同じ文字列でも残る
    assert_eq!(f.content(&message).await, "F1.txt");
    Ok(())
}

/// 取得を繰り返し落とした行を `failed` にするときも、終わったジョブの `processed_files` が合う
#[tokio::test]
async fn test_orphan_files_update_processed_files_when_giving_up_stopped_rows() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, |_| {}).await?;
    let job = f.job("completed").await?;
    f.add(Row {
        file: "F1",
        job: &job,
        content: "",
        in_content: false,
        size: 1,
    })
    .await?;
    sql(
        &f.state,
        "UPDATE slack_files SET lease_token='dead',lease_until=now()-interval '1 minute',attempts=3 WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    assert!(!f.once().await);
    assert_eq!(f.file("F1").await["status"], "failed");
    let jobs = sql(
        &f.state,
        "SELECT json_build_object('n',processed_files)::text FROM slack_import_jobs WHERE id=$1",
        &[&job],
    )
    .await?;
    assert_eq!(jobs[0]["n"], 1);
    Ok(())
}

// ---- ユーザートークンでの取得 (Bot に見えないファイル) ----

const USER_KEY: [u8; 32] = [7u8; 32];

/// OAuth と暗号化キーを入れ、許可 URL を `<サーバー>/` にする (Slack の本物の URL と同じ `/files-pri/…` を使うため)
fn with_links(files: &MockServer) -> impl FnMut(&mut Config) + use<> {
    use base64::Engine;
    let uri = files.uri();
    move |c| {
        c.slack.files_base_url = format!("{uri}/");
        c.slack.client_id = Some("cid".into());
        c.slack.client_secret = Some("sec".into());
        c.slack.team_id = None;
        c.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
        c.slack.token_encryption_key =
            Some(base64::engine::general_purpose::STANDARD.encode(USER_KEY));
    }
}

impl Fixture {
    /// Slack の `url_private_download` と同じ形 (`/files-pri/<team>-<file>/download/<file>.txt`) の行を作る
    async fn add_pri(&self, file: &str, job: &str) -> anyhow::Result<String> {
        let url = self.pri_url(file);
        self.add_url(
            Row {
                file,
                job,
                content: "",
                in_content: false,
                size: 1,
            },
            &url,
        )
        .await
    }

    fn pri_url(&self, file: &str) -> String {
        format!(
            "{}files-pri/{}-{file}/download/{file}.txt",
            self.state.config.slack.files_base_url, self.team
        )
    }

    fn pri_path(&self, file: &str) -> String {
        format!("/files-pri/{}-{file}/download/{file}.txt", self.team)
    }

    async fn extra_user(&self) -> anyhow::Result<String> {
        let app = create_router(self.state.clone());
        let user = uuid::Uuid::new_v4().to_string();
        let status = request(
            &app,
            "/api/users",
            &user,
            json!({"id": user, "user_name": format!("files_{}", uuid::Uuid::new_v4().simple()), "display_name": "files"}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        Ok(user)
    }

    /// 連携を作る。`team` は、別のワークスペースの連携を作るときに指定する
    async fn link(
        &self,
        user: &str,
        token: &str,
        scopes: &str,
        team: Option<&str>,
    ) -> anyhow::Result<()> {
        let enc = backend::slack::crypto::encrypt(&USER_KEY, token)?;
        sql(
            &self.state,
            "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc,scopes) \
             VALUES($1,$2,$3,$4,$5) RETURNING json_build_object('ok',true)::text",
            &[
                user,
                team.unwrap_or(&self.team),
                &format!("U{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                &enc,
                scopes,
            ],
        )
        .await?;
        Ok(())
    }
}

fn login_redirect() -> ResponseTemplate {
    ResponseTemplate::new(302).insert_header(
        "location",
        "https://zliworkspace.slack.com/?redir=%2Ffiles-pri%2FT-F%2Fdownload%2Fx",
    )
}

async fn mount_pri(
    files: &MockServer,
    f: &Fixture,
    file: &str,
    token: &str,
    resp: ResponseTemplate,
) {
    Mock::given(method("GET"))
        .and(path(f.pri_path(file)))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(resp)
        .mount(files)
        .await;
}

fn body(content: &[u8]) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/plain")
        .set_body_bytes(content.to_vec())
}

#[tokio::test]
async fn test_orphan_files_use_the_linked_user_token_when_the_bot_cannot_see_the_file()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    f.link(&f.user, "xoxp-alice", "channels:history,files:read", None)
        .await?;
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;
    mount_pri(&files, &f, "F1", "xoxp-alice", body(b"hello")).await;

    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "done");
    assert!(row["attachment"].is_string());
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_fail_at_once_when_no_token_can_see_the_file() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    f.link(&f.user, "xoxp-alice", "files:read", None).await?;
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;
    mount_pri(&files, &f, "F1", "xoxp-alice", login_redirect()).await;

    assert!(f.once().await);
    let row = f.file("F1").await;
    // 取り直しても変わらないので、3 回試さずに確定する
    assert_eq!(row["status"], "failed");
    assert_eq!(row["attempts"], 1);
    assert!(
        row["error"]
            .as_str()
            .unwrap()
            .contains("not visible to any linked user")
    );
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_retry_when_a_token_was_denied_and_another_failed() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    f.link(&f.user, "xoxp-alice", "files:read", None).await?;
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;
    mount_pri(&files, &f, "F1", "xoxp-alice", ResponseTemplate::new(503)).await;

    assert!(f.once().await);
    let row = f.file("F1").await;
    // 一時的な失敗が混じるときは、確定させない
    assert_eq!(row["status"], "pending");
    assert_eq!(row["attempts"], 1);
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_wait_without_counting_while_no_user_has_files_read() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    // `files:read` がない連携 (`remote_files:read` は別のスコープ) と、別のワークスペースの連携は、使わない
    f.link(
        &f.user,
        "xoxp-alice",
        "channels:history,remote_files:read",
        None,
    )
    .await?;
    let other = f.extra_user().await?;
    f.link(&other, "xoxp-other", "files:read", Some("TOTHER"))
        .await?;
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;
    mount_pri(&files, &f, "F1", "xoxp-alice", body(b"no")).await;
    mount_pri(&files, &f, "F1", "xoxp-other", body(b"no")).await;

    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "pending");
    assert_eq!(row["attempts"], 0);
    assert_eq!(row["waiting"], true);
    assert!(
        row["error"]
            .as_str()
            .unwrap()
            .contains("no linked user with files:read")
    );
    // 再連携 (files:read が付く) して待ちが終われば、取れる
    sql(
        &f.state,
        "UPDATE slack_user_links SET scopes='files:read' WHERE user_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.user],
    )
    .await?;
    f.expire_wait().await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_fail_a_bot_only_denial_when_links_are_not_enabled() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let uri = files.uri();
    let f = fixture(&files, move |c| {
        c.slack.files_base_url = format!("{uri}/");
        c.slack.client_id = None;
    })
    .await?;
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;

    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(row["status"], "failed");
    assert_eq!(row["attempts"], 1);
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_use_only_members_for_private_channels_and_try_every_member()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    sql(
        &f.state,
        "UPDATE channels SET is_private=true WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.channel],
    )
    .await?;
    // メンバーでない連携は使わない (取れる応答があっても)
    let outsider = f.extra_user().await?;
    f.link(&outsider, "xoxp-outsider", "files:read", None)
        .await?;
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;
    mount_pri(&files, &f, "F1", "xoxp-outsider", body(b"no")).await;
    assert!(f.once().await);
    let row = f.file("F1").await;
    assert_eq!(
        (row["status"].as_str(), row["attempts"].as_i64()),
        (Some("pending"), Some(0))
    );

    // メンバーが 4 人いて、先頭の 3 人に見えなくても、4 人目で取れる (人数で切らない)
    let mut members = vec![f.user.clone()];
    for _ in 0..3 {
        let m = f.extra_user().await?;
        sql(
            &f.state,
            "INSERT INTO channel_user(channel_id,user_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) \
             RETURNING json_build_object('ok',true)::text",
            &[&f.channel, &m],
        )
        .await?;
        members.push(m);
    }
    for (i, m) in members.iter().enumerate() {
        let token = format!("xoxp-m{i}");
        f.link(m, &token, "files:read", None).await?;
        let resp = if i == 3 {
            body(b"found")
        } else {
            login_redirect()
        };
        mount_pri(&files, &f, "F1", &token, resp).await;
    }
    f.expire_wait().await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_never_use_user_tokens_for_zip_rows_or_other_files_urls()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    f.link(&f.user, "xoxp-alice", "files:read", None).await?;
    // ZIP の取り込みの行は、ファイル ID と URL が一致していても、ユーザーのトークンでは取らない (Bot だけ)
    let job = f.job("completed").await?;
    f.add_pri("FZIP", &job).await?;
    mount_pri(&files, &f, "FZIP", "xoxb-test", login_redirect()).await;
    Mock::given(method("GET"))
        .and(path(f.pri_path("FZIP")))
        .and(header("authorization", "Bearer xoxp-alice"))
        .respond_with(body(b"no"))
        .expect(0)
        .mount(&files)
        .await;
    assert!(f.once().await);
    let row = f.file("FZIP").await;
    // 連携が増えても取れないので、待たずに確定する
    assert_eq!(row["status"], "failed");
    assert_eq!(row["attempts"], 1);

    // 差分同期の行でも、URL がその行のファイルのものでなければ、ユーザーのトークンは使わない
    f.add_url(
        Row {
            file: "F1",
            job: "",
            content: "",
            in_content: false,
            size: 1,
        },
        &format!(
            "{}files-pri/{}-OTHER/download/x.txt",
            f.state.config.slack.files_base_url, f.team
        ),
    )
    .await?;
    Mock::given(method("GET"))
        .and(path(format!("/files-pri/{}-OTHER/download/x.txt", f.team)))
        .and(header("authorization", "Bearer xoxp-alice"))
        .respond_with(body(b"no"))
        .expect(0)
        .mount(&files)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/files-pri/{}-OTHER/download/x.txt", f.team)))
        .and(header("authorization", "Bearer xoxb-test"))
        .respond_with(login_redirect())
        .mount(&files)
        .await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "failed");
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_skip_rows_of_jobs_without_a_bot_token() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let mut links = with_links(&files);
    let f = fixture(&files, move |c| {
        links(c);
        c.slack.import_bot_token = None;
    })
    .await?;
    f.link(&f.user, "xoxp-alice", "files:read", None).await?;
    // Bot なしで完了したジョブの行は、Bot を設定するまで未取得のまま残す (取らない、数えない)
    let job = f.job("completed").await?;
    f.add_pri("FZIP", &job).await?;
    assert!(!f.once().await);
    let row = f.file("FZIP").await;
    assert_eq!(
        (row["status"].as_str(), row["attempts"].as_i64()),
        (Some("pending"), Some(0))
    );

    // 差分同期の行は、ユーザーのトークンだけで取れる
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxp-alice", body(b"ok")).await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    Ok(())
}

#[tokio::test]
async fn test_orphan_files_accept_several_sync_sources_for_one_channel() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let files = MockServer::start().await;
    let f = fixture(&files, with_links(&files)).await?;
    f.link(&f.user, "xoxp-alice", "files:read", None).await?;
    // 同じ Zoo チャンネルに、Slack チャンネルが 2 つ対応し、どちらにも同期の行がある
    for slack in ["C1", "C2"] {
        sql(
            &f.state,
            "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id) VALUES($1,$2,$3) \
             RETURNING json_build_object('ok',true)::text",
            &[&f.team, slack, &f.channel],
        )
        .await?;
        sql(
            &f.state,
            "INSERT INTO slack_channel_syncs(team_id,slack_channel_id,token_user_id) VALUES($1,$2,$3) \
             RETURNING json_build_object('ok',true)::text",
            &[&f.team, slack, &f.user],
        )
        .await?;
    }
    f.add_pri("F1", "").await?;
    mount_pri(&files, &f, "F1", "xoxb-test", login_redirect()).await;
    mount_pri(&files, &f, "F1", "xoxp-alice", body(b"ok")).await;
    assert!(f.once().await);
    assert_eq!(f.file("F1").await["status"], "done");
    Ok(())
}
