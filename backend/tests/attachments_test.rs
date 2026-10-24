use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    config::Config, db::init_db, models::Attachment, routes::create_router, state::AppState,
    storage::ensure_bucket_exists,
};

#[tokio::test]
async fn test_attachments_presign_and_validation() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    // 1. ユーザー作成
    for (id, name) in [(&alice_id, "alice_att"), (&bob_id, "bob_att")] {
        let user_payload = json!({
            "id": id,
            "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
            "display_name": name,
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 2. 公開チャンネル作成 (Alice が作成、Alice はメンバー)
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("att_pub_{}", uuid::Uuid::new_v4().simple()),
        "description": "Attachments Public",
        "is_private": false,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. 正常系: Presigned PUT URL 発行 (Alice)
    let presign_payload = json!({
        "channel_id": channel_id,
        "file_name": "test_image.png",
        "file_size": 1024,
        "content_type": "image/png"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&presign_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let res: Value = serde_json::from_slice(&body)?;
    let attachment_id = res["data"]["attachment_id"].as_str().unwrap().to_string();
    let upload_url = res["data"]["upload_url"].as_str().unwrap();
    let storage_key = res["data"]["storage_key"].as_str().unwrap();

    assert!(!attachment_id.is_empty());
    assert!(upload_url.contains("test_image.png") || upload_url.contains(&attachment_id));
    assert_eq!(
        storage_key,
        format!("attachments/{channel_id}/{attachment_id}")
    );

    // 4. 異常系: 危険な MIME タイプは 400 Bad Request
    let dangerous_payload = json!({
        "channel_id": channel_id,
        "file_name": "malicious.exe",
        "file_size": 2048,
        "content_type": "application/x-msdownload"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dangerous_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 4b. 異常系: 不正な MIME フォーマット (スラッシュなし、制御文字混入等)
    for bad_mime in [
        "invalid",
        "image/",
        "/png",
        "image/png\r\nX-Bad: true",
        "image/png; invalid token @!",
    ] {
        let bad_mime_payload = json!({
            "channel_id": channel_id,
            "file_name": "test.png",
            "file_size": 1024,
            "content_type": bad_mime
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/attachments/presign")
            .header("content-type", "application/json")
            .header("X-Dev-User", &alice_id)
            .body(Body::from(serde_json::to_vec(&bad_mime_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // 4c. 認可検証: 公開チャンネルでも未参加のユーザー (Bob) は 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&presign_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Bob がチャンネルに参加した後は presign 可能になること
    let join_req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/join"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(join_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&presign_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. 異常系: サイズ超過 (50MB超) は 400 Bad Request
    let oversized_payload = json!({
        "channel_id": channel_id,
        "file_name": "huge.zip",
        "file_size": 100 * 1024 * 1024, // 100MB
        "content_type": "application/zip"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&oversized_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. 異常系: 未参加のプライベートチャンネルへの発行は 403 Forbidden
    let priv_channel_id = uuid::Uuid::new_v4().to_string();
    let priv_payload = json!({
        "id": priv_channel_id,
        "channel_name": format!("secret_{}", uuid::Uuid::new_v4().simple()),
        "description": "Secret",
        "is_private": true,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&priv_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let bob_presign_payload = json!({
        "channel_id": priv_channel_id,
        "file_name": "test_image.png",
        "file_size": 1024,
        "content_type": "image/png"
    });

    let bob_req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&bob_presign_payload)?))?;

    let resp = app.clone().oneshot(bob_req).await?;
    // プライベートチャンネルに未所属の場合は NotFound または Forbidden
    assert!(resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn test_attachments_upload_verification_and_message_linking() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    for (id, name) in [(&alice_id, "alice_link"), (&bob_id, "bob_link")] {
        let user_payload = json!({
            "id": id,
            "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
            "display_name": name,
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("link_chan_{}", uuid::Uuid::new_v4().simple()),
        "description": "Attachment Linking",
        "is_private": false,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. Alice が presign 要求
    let file_content = b"fake png file content for test";
    let presign_payload = json!({
        "channel_id": channel_id,
        "file_name": "photo.png",
        "file_size": file_content.len() as i64,
        "content_type": "image/png"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&presign_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let res: Value = serde_json::from_slice(&body)?;
    let attachment_id = res["data"]["attachment_id"].as_str().unwrap().to_string();
    let storage_key = res["data"]["storage_key"].as_str().unwrap().to_string();

    // 1b. 未紐付け (message_id IS NULL) の状態での GET /url 認可検証:
    // - アップロード本人 (Alice) は 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{attachment_id}/url"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // - 他人 (Bob) は存在秘匿のため 404 Not Found
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{attachment_id}/url"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 2. 実体をアップロードせずにメッセージへ紐付けようとすると 400 Bad Request
    let message_id = uuid::Uuid::new_v4().to_string();
    let msg_payload_unverified = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Look at this image without upload",
        "attachment_ids": [attachment_id]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg_payload_unverified)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 2b. 上限 (10件) を超える 11 件の attachment_ids は 400 Bad Request
    let too_many_att_ids: Vec<String> = (0..11).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let msg_payload_too_many = json!({
        "channel_id": channel_id,
        "content": "Message with too many attachments",
        "attachment_ids": too_many_att_ids
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg_payload_too_many)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 3. S3 (RustFS) に実体を直接 PutObject する
    state
        .s3_client
        .put_object()
        .bucket(&config.s3.bucket)
        .key(&storage_key)
        .content_type("image/png")
        .body(file_content.to_vec().into())
        .send()
        .await?;

    // 4. Bob が Alice のアップロードした attachment_id を勝手にメッセージに紐付けようとすると 403 Forbidden
    let bob_msg_payload = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "channel_id": channel_id,
        "content": "Bob stealing attachment",
        "attachment_ids": [attachment_id]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&bob_msg_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 5. Alice 本人がメッセージを投稿して紐付け (成功)
    let msg_payload_valid = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Valid message with attachment",
        "attachment_ids": [attachment_id]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg_payload_valid)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 6. DB 上で attachments.message_id が設定されていることを検証
    let mut db_conn = db.clone();
    let att_record = Attachment::get_by_id(&mut db_conn, &attachment_id).await?;
    assert_eq!(att_record.message_id, Some(message_id.clone()));

    // 7. GET /api/attachments/{id}/url で署名付き GET URL を取得
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{attachment_id}/url"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let res: Value = serde_json::from_slice(&body)?;
    let download_url = res["data"]["download_url"].as_str().unwrap();
    assert!(download_url.contains("response-content-disposition"));
    // 画像なので inline が含まれていること
    assert!(download_url.contains("inline"));

    // 8. 親メッセージを論理削除した場合、GET URL は 404 Not Found になること
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{attachment_id}/url"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 9. 画像以外のファイル (例: PDF) では Content-Disposition が attachment になること
    let pdf_payload = json!({
        "channel_id": channel_id,
        "file_name": "manual.pdf",
        "file_size": 1024,
        "content_type": "application/pdf"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&pdf_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let res: Value = serde_json::from_slice(&body)?;
    let pdf_attachment_id = res["data"]["attachment_id"].as_str().unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{pdf_attachment_id}/url"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let res: Value = serde_json::from_slice(&body)?;
    let download_url = res["data"]["download_url"].as_str().unwrap();
    assert!(download_url.contains("attachment"));
    assert!(!download_url.contains("inline"));

    Ok(())
}
