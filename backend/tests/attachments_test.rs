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
    let charlie_id = uuid::Uuid::new_v4().to_string();

    // 1. ユーザー作成
    for (id, name) in [
        (&alice_id, "alice_att"),
        (&bob_id, "bob_att"),
        (&charlie_id, "charlie_att"),
    ] {
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

    // 4b-2. 異常系: ファイル名に制御文字 (\0, \r, \n) が含まれる場合は 400 Bad Request
    for bad_name in [
        "bad\0name.png",
        "bad\rname.png",
        "bad\nname.png",
        "\r\nbad.png",
    ] {
        let bad_name_payload = json!({
            "channel_id": channel_id,
            "file_name": bad_name,
            "file_size": 1024,
            "content_type": "image/png"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/attachments/presign")
            .header("content-type", "application/json")
            .header("X-Dev-User", &alice_id)
            .body(Body::from(serde_json::to_vec(&bad_name_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "control character in file_name must be rejected: {bad_name:?}"
        );
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

    // 6. 異常系: 未参加のプライベートチャンネルへの発行は厳密に 404 Not Found (存在隠蔽)
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
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 7. アーカイブ済みプライベートチャンネル: メンバー (Alice) は 400 Bad Request、非メンバー (Bob) は 404 Not Found
    let archive_priv_req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{priv_channel_id}/archive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(archive_priv_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let alice_archived_priv_req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&bob_presign_payload)?))?;
    let resp = app.clone().oneshot(alice_archived_priv_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let bob_archived_priv_req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&bob_presign_payload)?))?;
    let resp = app.clone().oneshot(bob_archived_priv_req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 8. アーカイブ済み公開チャンネル: 未参加 (Charlie) は 403 Forbidden、メンバー (Alice) は 400 Bad Request
    let archive_pub_req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(archive_pub_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let charlie_archived_pub_req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &charlie_id)
        .body(Body::from(serde_json::to_vec(&presign_payload)?))?;
    let resp = app.clone().oneshot(charlie_archived_pub_req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let alice_archived_pub_req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&presign_payload)?))?;
    let resp = app.clone().oneshot(alice_archived_pub_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

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

#[tokio::test]
async fn test_presigned_put_signature_enforcement_and_attachment_only_message() -> anyhow::Result<()>
{
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let user_payload = json!({
        "id": alice_id,
        "user_name": format!("alice_put_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Alice Put",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);

    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("put_chan_{}", uuid::Uuid::new_v4().simple()),
        "description": "Presigned PUT Signature Test",
        "is_private": false,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);

    // 1. Presigned PUT URL を取得
    let file_bytes = b"\x89PNG\r\n\x1a\npresigned-signature-test-payload";
    let presign_payload = json!({
        "channel_id": channel_id,
        "file_name": "signed_image.png",
        "file_size": file_bytes.len() as i64,
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
    let upload_url = res["data"]["upload_url"].as_str().unwrap().to_string();

    let http_client = reqwest::Client::new();

    // 2. 異なる Content-Type での実 HTTP PUT は署名不一致で 403 Forbidden
    let wrong_ct_resp = http_client
        .put(&upload_url)
        .header("content-type", "text/plain")
        .body(file_bytes.to_vec())
        .send()
        .await?;
    assert_eq!(
        wrong_ct_resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "PUT with mismatched Content-Type must be rejected with 403"
    );

    // 3. 異なるサイズ (ボディ長) での実 HTTP PUT は署名不一致で 403 Forbidden
    let mut wrong_size_bytes = file_bytes.to_vec();
    wrong_size_bytes.extend_from_slice(b"-extra-bytes");
    let wrong_len_resp = http_client
        .put(&upload_url)
        .header("content-type", "image/png")
        .body(wrong_size_bytes)
        .send()
        .await?;
    assert_eq!(
        wrong_len_resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "PUT with mismatched Content-Length must be rejected with 403"
    );

    // 4. 正しい Content-Type とサイズでの実 HTTP PUT は 200 OK
    let valid_put_resp = http_client
        .put(&upload_url)
        .header("content-type", "image/png")
        .body(file_bytes.to_vec())
        .send()
        .await?;
    assert_eq!(valid_put_resp.status(), reqwest::StatusCode::OK);

    // 5. 添付なしの空文字本文は 400 Bad Request
    let empty_no_att_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "",
        }))?))?;
    let resp = app.clone().oneshot(empty_no_att_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. 添付ありの空文字本文 (content: "") は 200 OK で投稿できること
    let msg_id = uuid::Uuid::new_v4().to_string();
    let empty_with_att_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": msg_id,
            "channel_id": channel_id,
            "content": "",
            "attachment_ids": [attachment_id],
        }))?))?;
    let resp = app.clone().oneshot(empty_with_att_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let msg_body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    assert_eq!(msg_body["data"]["content"], "");

    // 6b. 添付ありメッセージを一度テキスト付きに更新し、再度空文字本文に編集しても 200 OK になること
    let edit_text_req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/messages/{msg_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "content": "Added caption",
        }))?))?;
    assert_eq!(
        app.clone().oneshot(edit_text_req).await?.status(),
        StatusCode::OK
    );

    let edit_empty_req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/messages/{msg_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "content": "",
        }))?))?;
    assert_eq!(
        app.clone().oneshot(edit_empty_req).await?.status(),
        StatusCode::OK
    );

    // 7. download_url を取得して実 HTTP GET を行い、200 OK・ボディ一致・inline Content-Disposition を検証
    let get_url_req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{attachment_id}/url"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(get_url_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let url_body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    let download_url = url_body["data"]["download_url"].as_str().unwrap();

    let download_resp = http_client.get(download_url).send().await?;
    assert_eq!(download_resp.status(), reqwest::StatusCode::OK);
    let content_disp = download_resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_disp.starts_with("inline;"),
        "expected inline Content-Disposition, got: {content_disp}"
    );
    let downloaded_bytes = download_resp.bytes().await?;
    assert_eq!(downloaded_bytes.as_ref(), file_bytes);

    Ok(())
}

#[tokio::test]
async fn test_attachment_expiry_is_timezone_independent() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let sep = if config.database_url.contains('?') {
        '&'
    } else {
        '?'
    };
    config.database_url = format!(
        "{}{}options=-c%20timezone%3DAsia/Tokyo",
        config.database_url, sep
    );

    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;

    // セッション TimeZone が Asia/Tokyo になっていることを確認
    {
        let mut check_db = db.clone();
        let mut tx = check_db.transaction().await?;
        let rows = toasty::sql::query("SELECT current_setting('TimeZone')")
            .exec(&mut tx)
            .await?;
        let tz = rows
            .into_iter()
            .next()
            .and_then(|r| match r {
                toasty::stmt::Value::Record(mut rec) => rec.fields.pop(),
                _ => None,
            })
            .and_then(|v| match v {
                toasty::stmt::Value::String(s) => Some(s),
                _ => None,
            });
        assert_eq!(tz.as_deref(), Some("Asia/Tokyo"));
        tx.commit().await?;
    }

    let alice_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "id": alice_id,
            "user_name": format!("alice_tz_{}", uuid::Uuid::new_v4().simple()),
            "display_name": "Alice TZ",
        }))?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);

    let channel_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": channel_id,
            "channel_name": format!("tz_chan_{}", uuid::Uuid::new_v4().simple()),
            "description": "TZ Test",
            "is_private": false,
        }))?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);

    let file_content = b"tz-test-bytes";
    let mut created_atts = Vec::new();
    for name in ["valid_16h.png", "expired_24h.png"] {
        let req = Request::builder()
            .method("POST")
            .uri("/api/attachments/presign")
            .header("content-type", "application/json")
            .header("X-Dev-User", &alice_id)
            .body(Body::from(serde_json::to_vec(&json!({
                "channel_id": channel_id,
                "file_name": name,
                "file_size": file_content.len() as i64,
                "content_type": "image/png"
            }))?))?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
        let att_id = body["data"]["attachment_id"].as_str().unwrap().to_string();
        let storage_key = body["data"]["storage_key"].as_str().unwrap().to_string();

        state
            .s3_client
            .put_object()
            .bucket(&config.s3.bucket)
            .key(&storage_key)
            .content_type("image/png")
            .body(file_content.to_vec().into())
            .send()
            .await?;

        created_atts.push(att_id);
    }

    let valid_16h_id = &created_atts[0];
    let expired_24h_id = &created_atts[1];

    // DB の created_at をそれぞれ UTC 基準で 16 時間前・24 時間前に設定
    {
        let mut update_db = db.clone();
        let mut tx = update_db.transaction().await?;
        toasty::sql::query(
            "UPDATE attachments
             SET created_at = (now() AT TIME ZONE 'UTC') - INTERVAL '16 hours'
             WHERE id = $1
             RETURNING id",
        )
        .bind(valid_16h_id)
        .exec(&mut tx)
        .await?;

        toasty::sql::query(
            "UPDATE attachments
             SET created_at = (now() AT TIME ZONE 'UTC') - INTERVAL '24 hours'
             WHERE id = $1
             RETURNING id",
        )
        .bind(expired_24h_id)
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    // 1. 16 時間前の添付 (23 時間以内) は TimeZone=Asia/Tokyo でも 200 OK で紐付け成功する
    let ok_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "16h old attachment should succeed",
            "attachment_ids": [valid_16h_id],
        }))?))?;
    let resp = app.clone().oneshot(ok_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. 24 時間前の添付 (23 時間超過) は 400 Bad Request で拒否される
    let exp_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "24h old attachment should expire",
            "attachment_ids": [expired_24h_id],
        }))?))?;
    let resp = app.clone().oneshot(exp_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn test_attachments_presign_and_get_with_public_endpoint() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.s3.public_endpoint = Some("https://attachments.zliusercontent.com".to_string());

    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;

    let alice_id = uuid::Uuid::new_v4().to_string();

    // 1. ユーザー作成
    let user_payload = json!({
        "id": alice_id,
        "user_name": format!("alice_pub_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Alice Pub",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. チャンネル作成
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("pub_end_{}", uuid::Uuid::new_v4().simple()),
        "description": "Public Endpoint Test",
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

    // 3. presign API の upload_url が S3_PUBLIC_ENDPOINT に書き換えられていることを検証
    let presign_req = Request::builder()
        .method("POST")
        .uri("/api/attachments/presign")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "file_name": "photo.png",
            "file_size": 1024,
            "content_type": "image/png",
        }))?))?;

    let resp = app.clone().oneshot(presign_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    let upload_url = body["data"]["upload_url"].as_str().unwrap();
    let attachment_id = body["data"]["attachment_id"].as_str().unwrap();

    assert!(
        upload_url.starts_with("https://attachments.zliusercontent.com/"),
        "upload_url should start with public endpoint, got {upload_url}"
    );
    assert!(
        upload_url.contains("X-Amz-Signature="),
        "upload_url should preserve SigV4 query parameters"
    );

    // 4. GET URL API の download_url も S3_PUBLIC_ENDPOINT に書き換えられていることを検証
    let get_url_req = Request::builder()
        .method("GET")
        .uri(format!("/api/attachments/{attachment_id}/url"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(get_url_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    let download_url = body["data"]["download_url"].as_str().unwrap();

    assert!(
        download_url.starts_with("https://attachments.zliusercontent.com/"),
        "download_url should start with public endpoint, got {download_url}"
    );
    assert!(
        download_url.contains("X-Amz-Signature="),
        "download_url should preserve SigV4 query parameters"
    );

    Ok(())
}

#[tokio::test]
async fn channel_image_metadata_is_generated_and_returned_to_home() -> anyhow::Result<()> {
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(12, 8).write_to(&mut image, image::ImageFormat::Png)?;
    let bytes = image.into_inner();
    let storage = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("HEAD"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .insert_header("content-length", bytes.len().to_string()),
        )
        .mount(&storage)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
        .mount(&storage)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.s3.endpoint = storage.uri();
    config.s3.force_path_style = true;
    config.s3.access_key_id = "test".into();
    config.s3.secret_access_key = "test".into();
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let user = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let attachment = uuid::Uuid::new_v4().to_string();
    let message = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$1,$2,false)",
    )
    .bind(channel.as_str())
    .bind(user.as_str())
    .exec(&mut db)
    .await?;
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
    )
    .bind(user.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    toasty::sql::statement("INSERT INTO attachments(id,uploader_id,channel_id,file_name,file_size,content_type,storage_key) VALUES($1,$2,$3,'photo.png',$4::bigint,'image/png','photo')")
        .bind(attachment.as_str()).bind(user.as_str()).bind(channel.as_str()).bind(bytes.len().to_string()).exec(&mut db).await?;
    let response = app.clone().oneshot(Request::builder().method("POST").uri("/api/messages")
        .header("content-type","application/json").header("X-Dev-User", &user)
        .body(Body::from(json!({"id":message,"channel_id":channel,"content":"photo","attachment_ids":[attachment]}).to_string()))?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let saved = Attachment::get_by_id(&mut db, attachment.as_str()).await?;
    assert_eq!(saved.width, Some(12));
    assert_eq!(saved.height, Some(8));
    assert!(blurhash::decode(saved.blurhash.as_deref().unwrap(), 2, 2, 1.0).is_ok());
    toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,channel_id) VALUES($1,$2,'photo',$3) ON CONFLICT DO NOTHING")
        .bind(message.as_str()).bind(user.as_str()).bind(channel.as_str()).exec(&mut db).await?;
    assert_eq!(
        storage
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() == "GET")
            .count(),
        1
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/home/{message}"))
                .header("X-Dev-User", &user)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let post: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(post["attachments"][0]["width"], 12);
    assert_eq!(post["attachments"][0]["height"], 8);
    assert_eq!(post["media"][0]["blurhash"], saved.blurhash.unwrap());
    toasty::sql::statement("DELETE FROM attachments WHERE id=$1")
        .bind(attachment.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("DELETE FROM home_posts WHERE id=$1")
        .bind(message.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("DELETE FROM messages WHERE id=$1")
        .bind(message.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    // App deliveries retain references to this channel and its creator. Use the
    // production soft-delete lifecycle rather than erasing delivery history.
    toasty::sql::statement("UPDATE channels SET deleted_at=now() AT TIME ZONE 'UTC' WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    Ok(())
}
