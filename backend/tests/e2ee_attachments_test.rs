//! 暗号化した添付（#364）の結合テスト。仕様は docs/e2ee.md の「暗号化した添付」。
//!
//! 完了条件の「S3 に平文の添付が残らない」を、実際の S3（開発用の RustFS）へのアップロードと取得で確かめる。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config, db::init_db, routes::create_router, state::AppState,
    storage::ensure_bucket_exists,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::{
    DeviceAccount, ENCRYPTED_ATTACHMENT_CONTENT_TYPE, ENCRYPTED_ATTACHMENT_FILE_NAME,
    decrypt_attachment, encrypt_attachment,
};

/// 平文に入れる目印。S3 のどのオブジェクトにも現れてはいけない。
const MARKER: &[u8] = b"ZOO2-E2EE-364-PLAINTEXT-MARKER";

struct Env {
    app: Router,
    db: toasty::Db,
    state: AppState,
}

async fn env() -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await.unwrap();
    let state = AppState::new(db.clone(), config);
    ensure_bucket_exists(&state.s3_client, &state.config.s3.bucket).await;
    Env {
        app: create_router(state.clone()),
        db,
        state,
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

/// 画像として読める平文（PNG の先頭）に目印を入れたもの。
fn plaintext() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    for _ in 0..64 {
        bytes.extend_from_slice(MARKER);
    }
    bytes
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

impl Env {
    async fn call(
        &self,
        method: &str,
        path: &str,
        actor: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("X-Dev-User", actor)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn query(&self, sql: &str, params: &[&str]) -> Vec<Value> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let rows = backend::apps::query(&mut tx, sql, params).await.unwrap();
        tx.commit().await.unwrap();
        rows
    }

    async fn exec(&self, sql: &str, params: &[&str]) -> Result<u64, toasty::Error> {
        let mut statement = toasty::sql::statement(sql);
        for p in params {
            statement = statement.bind(*p);
        }
        let mut db = self.db.clone();
        statement.exec(&mut db).await
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/users",
                &id,
                json!({"id": id, "user_name": format!("e2ee_att_{}", Uuid::new_v4().simple()), "display_name": "E2EE"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    }

    async fn channel(&self, creator: &str, members: &[&str], encrypted: bool) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("att-{}", Uuid::new_v4().simple()), "is_private": true, "encrypted": encrypted}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for member in members {
            let (status, body) = self
                .call(
                    "POST",
                    &format!("/api/channels/{id}/users"),
                    creator,
                    json!({"user_id": member}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        id
    }

    async fn device(&self, user: &str) -> DeviceAccount {
        let account = DeviceAccount::new(user, &Uuid::new_v4().to_string()).unwrap();
        let (status, body) = self
            .call(
                "POST",
                "/api/e2ee/devices",
                user,
                json!({"device_keys": account.device_keys(), "display_name": "Test"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        account
    }

    async fn epoch(&self, channel: &str) -> i64 {
        self.query(
            "SELECT membership_epoch::text FROM channels WHERE id = $1",
            &[channel],
        )
        .await[0]
            .as_i64()
            .unwrap()
    }

    async fn presign(
        &self,
        user: &str,
        channel: &str,
        file_name: &str,
        content_type: &str,
        size: usize,
    ) -> (StatusCode, Value) {
        self.call(
            "POST",
            "/api/attachments/presign",
            user,
            json!({"channel_id": channel, "file_name": file_name, "file_size": size, "content_type": content_type}),
        )
        .await
    }

    async fn s3_object(&self, key: &str) -> Vec<u8> {
        let object = self
            .state
            .s3_client
            .get_object()
            .bucket(&self.state.config.s3.bucket)
            .key(key)
            .send()
            .await
            .unwrap();
        object.body.collect().await.unwrap().into_bytes().to_vec()
    }

    /// `attachments/<channel>/` の下のすべてのオブジェクト。
    async fn s3_objects(&self, channel: &str) -> Vec<(String, Vec<u8>)> {
        let listed = self
            .state
            .s3_client
            .list_objects_v2()
            .bucket(&self.state.config.s3.bucket)
            .prefix(format!("attachments/{channel}/"))
            .send()
            .await
            .unwrap();
        let mut objects = Vec::new();
        for object in listed.contents() {
            let key = object.key().unwrap().to_owned();
            let bytes = self.s3_object(&key).await;
            objects.push((key, bytes));
        }
        objects
    }
}

fn message(channel: &str, account: &DeviceAccount, epoch: i64) -> Value {
    json!({
        "id": Uuid::new_v4().to_string(),
        "channel_id": channel,
        "content": "",
        "epoch": epoch,
        "revision": 0,
        "ciphertext": {
            "algorithm": "m.megolm.v1.aes-sha2",
            "sender_key": account.identity_keys().curve25519.to_base64(),
            "device_id": account.device_id(),
            "session_id": "session",
            "ciphertext": "c",
        },
    })
}

/// 暗号化した添付を上げて送り、受け取った人が取得して復号できる。S3 と DB の行には平文が残らない。
#[tokio::test]
async fn encrypted_attachments_leave_no_plaintext_in_s3() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.channel(&alice, &[&bob], true).await;
    let device = env.device(&alice).await;
    let plaintext = plaintext();
    let encrypted = encrypt_attachment(&plaintext);

    // 本当のファイル名や MIME はサーバーに渡せない。
    let (status, body) = env
        .presign(&alice, &channel, "secret.png", "image/png", plaintext.len())
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN"),
        "{body}"
    );
    let (status, body) = env
        .presign(
            &alice,
            &channel,
            ENCRYPTED_ATTACHMENT_FILE_NAME,
            "image/png",
            plaintext.len(),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN"),
        "{body}"
    );

    let (status, body) = env
        .presign(
            &alice,
            &channel,
            ENCRYPTED_ATTACHMENT_FILE_NAME,
            ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
            encrypted.ciphertext.len(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let attachment_id = body["data"]["attachment_id"].as_str().unwrap().to_owned();
    let storage_key = body["data"]["storage_key"].as_str().unwrap().to_owned();
    let upload_url = body["data"]["upload_url"].as_str().unwrap().to_owned();
    let put = reqwest::Client::new()
        .put(&upload_url)
        .header("content-type", ENCRYPTED_ATTACHMENT_CONTENT_TYPE)
        .body(encrypted.ciphertext.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::OK);

    // センシティブの印は暗号化した本文に入れるので、平文の項目では受け付けない。
    let epoch = env.epoch(&channel).await;
    let mut sensitive = message(&channel, &device, epoch);
    sensitive["attachment_ids"] = json!([attachment_id]);
    sensitive["sensitive_attachment_ids"] = json!([attachment_id]);
    let (status, body) = env.call("POST", "/api/messages", &alice, sensitive).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN"),
        "{body}"
    );

    // 平文のチャンネルで上げた添付（平文の PNG）は、暗号化チャンネルのメッセージに紐付けられない。
    let plain_channel = env.channel(&alice, &[&bob], false).await;
    let (status, body) = env
        .presign(
            &alice,
            &plain_channel,
            "photo.png",
            "image/png",
            plaintext.len(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plain_attachment = body["data"]["attachment_id"].as_str().unwrap().to_owned();
    let mut cross = message(&channel, &device, epoch);
    cross["attachment_ids"] = json!([plain_attachment]);
    let (status, body) = env.call("POST", "/api/messages", &alice, cross).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        env.query(
            "SELECT (message_id IS NULL)::text FROM attachments WHERE id = $1",
            &[&plain_attachment],
        )
        .await,
        vec![json!(true)]
    );

    let mut send = message(&channel, &device, epoch);
    send["attachment_ids"] = json!([attachment_id]);
    let message_id = send["id"].as_str().unwrap().to_owned();
    let (status, body) = env.call("POST", "/api/messages", &alice, send).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 行には決まったファイル名と Content-Type だけがあり、画像の情報はない（画像として読まない）。
    let row = env
        .query(
            "SELECT json_build_object('message_id', message_id, 'file_name', file_name, 'content_type', content_type, 'file_size', file_size, 'width', width, 'height', height, 'blurhash', blurhash)::text FROM attachments WHERE id = $1",
            &[&attachment_id],
        )
        .await
        .remove(0);
    assert_eq!(
        row,
        json!({
            "message_id": message_id,
            "file_name": ENCRYPTED_ATTACHMENT_FILE_NAME,
            "content_type": ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
            "file_size": encrypted.ciphertext.len(),
            "width": null,
            "height": null,
            "blurhash": null,
        })
    );

    // S3 にあるのは暗号文だけ。
    let stored = env.s3_object(&storage_key).await;
    assert_eq!(stored, encrypted.ciphertext);
    for (key, bytes) in env.s3_objects(&channel).await {
        assert!(!contains(&bytes, MARKER), "{key} に平文が残っている");
        assert!(
            !contains(&bytes, b"\x89PNG"),
            "{key} に平文の PNG の先頭が残っている"
        );
    }

    // 受け取った人は取得して、本文の鍵で復号できる。
    let (status, body) = env
        .call(
            "GET",
            &format!("/api/attachments/{attachment_id}/url"),
            &bob,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["content_type"],
        ENCRYPTED_ATTACHMENT_CONTENT_TYPE
    );
    let download_url = body["data"]["download_url"].as_str().unwrap();
    let downloaded = reqwest::get(download_url)
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(
        decrypt_attachment(&encrypted.file, &downloaded).unwrap(),
        plaintext
    );
}

/// API 以外の経路で、暗号化チャンネルに平文の添付の行を作れない（DB のトリガー）。
#[tokio::test]
async fn the_database_rejects_plaintext_attachment_rows_in_encrypted_channels() {
    let env = env().await;
    let alice = env.user().await;
    let encrypted = env.channel(&alice, &[], true).await;
    let plain = env.channel(&alice, &[], false).await;
    let insert = "INSERT INTO attachments(id, uploader_id, channel_id, file_name, file_size, content_type, storage_key) VALUES ($1, $2, $3, $4, 1, $5, $6)";

    let ok = Uuid::new_v4().to_string();
    let key = format!("attachments/{encrypted}/{ok}");
    env.exec(
        insert,
        &[
            &ok,
            &alice,
            &encrypted,
            ENCRYPTED_ATTACHMENT_FILE_NAME,
            ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
            &key,
        ],
    )
    .await
    .unwrap();

    for (file_name, content_type, storage_key) in [
        ("secret.png", ENCRYPTED_ATTACHMENT_CONTENT_TYPE, None),
        (ENCRYPTED_ATTACHMENT_FILE_NAME, "image/png", None),
        (
            ENCRYPTED_ATTACHMENT_FILE_NAME,
            ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
            Some("home-images/x".to_owned()),
        ),
        (
            ENCRYPTED_ATTACHMENT_FILE_NAME,
            ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
            Some(format!("attachments/{plain}/x")),
        ),
    ] {
        let id = Uuid::new_v4().to_string();
        let storage_key = storage_key.unwrap_or_else(|| format!("attachments/{encrypted}/{id}"));
        let result = env
            .exec(
                insert,
                &[
                    &id,
                    &alice,
                    &encrypted,
                    file_name,
                    content_type,
                    &storage_key,
                ],
            )
            .await;
        assert!(
            result.is_err(),
            "{file_name} {content_type} {storage_key} を受け付けた"
        );
    }
    assert!(
        env.exec(
            "UPDATE attachments SET width = 1, height = 1 WHERE id = $1",
            &[&ok]
        )
        .await
        .is_err()
    );
    assert!(
        env.exec(
            "UPDATE attachments SET blurhash = 'x' WHERE id = $1",
            &[&ok]
        )
        .await
        .is_err()
    );
    // 決まった値や置き場所を、あとから書き換えることもできない。
    for update in [
        "UPDATE attachments SET file_name = 'secret.png' WHERE id = $1",
        "UPDATE attachments SET content_type = 'image/png' WHERE id = $1",
        "UPDATE attachments SET storage_key = 'home-images/x' WHERE id = $1",
    ] {
        assert!(
            env.exec(update, &[&ok]).await.is_err(),
            "{update} を受け付けた"
        );
    }

    // 平文のチャンネルは今までどおり。
    let id = Uuid::new_v4().to_string();
    env.exec(
        insert,
        &[
            &id,
            &alice,
            &plain,
            "photo.png",
            "image/png",
            &format!("attachments/{plain}/{id}"),
        ],
    )
    .await
    .unwrap();
    // 平文の行を、暗号化チャンネルに移すこともできない。
    assert!(
        env.exec(
            "UPDATE attachments SET channel_id = $2 WHERE id = $1",
            &[&id, &encrypted]
        )
        .await
        .is_err()
    );
}
