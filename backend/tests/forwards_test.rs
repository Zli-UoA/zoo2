//! メッセージ転送（引用）機能（Issue #690）の API・権限オラクル防止・E2EE 拒否・通知・Slack 連携バックフィルの統合テスト。

#[cfg(unix)]
use std::io::Write;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header},
};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};
#[cfg(unix)]
use zip::write::SimpleFileOptions;

#[cfg(unix)]
use backend::slack::worker;
use backend::{
    apps::{one, query},
    config::Config,
    db::init_db,
    routes::create_router,
    routes::forwards::{MAX_FORWARDED_MESSAGES_USER_POST, MAX_RESOLVE_FORWARD_IDS},
    slack::{self, backfill_forwards, sync},
    state::AppState,
};

/// `sync::run_once` / `worker::run_once` は DB 全体からジョブを拾うため、Slack テスト群は直列実行する。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn dev_config() -> Config {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    config
}

async fn sql_exec(
    state: &AppState,
    statement: &str,
    params: &[&str],
) -> anyhow::Result<Vec<Value>> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn sql_one(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<Value> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let row = one(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(row)
}

async fn sql_raw_err(state: &AppState, statement: &str, params: &[&str]) -> Result<(), String> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await.map_err(|e| e.to_string())?;
    let mut stmt = toasty::sql::statement(statement);
    for p in params {
        stmt = stmt.bind(*p);
    }
    match stmt.exec(&mut tx).await {
        Ok(_) => {
            tx.commit().await.map_err(|e| e.to_string())?;
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}

async fn call_with_headers(
    app: &Router,
    method_name: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, axum::http::HeaderMap, Value)> {
    let mut builder = Request::builder().method(method_name).uri(uri);
    builder = if user.starts_with("zoo_at_") {
        builder.header("Authorization", format!("Bearer {user}"))
    } else {
        builder.header("X-Dev-User", user)
    };
    let payload = if let Some(json_body) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&json_body)?)
    } else {
        Body::empty()
    };
    let response = app.clone().oneshot(builder.body(payload)?).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Ok((status, headers, parsed))
}

async fn call(
    app: &Router,
    method_name: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let (status, _, value) = call_with_headers(app, method_name, uri, user, body).await?;
    Ok((status, value))
}

async fn create_user(app: &Router, state: &AppState, prefix: &str) -> anyhow::Result<String> {
    let id = Uuid::new_v4().to_string();
    let (status, body) = call(
        app,
        "POST",
        "/api/users",
        &id,
        Some(json!({
            "id": id,
            "user_name": format!("{prefix}_{}", Uuid::new_v4().simple()),
            "display_name": prefix,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "create_user failed: {body}");
    let _ = state;
    Ok(id)
}

async fn create_channel(
    app: &Router,
    owner: &str,
    is_private: bool,
    encrypted: bool,
    members: &[&str],
) -> anyhow::Result<String> {
    let id = Uuid::new_v4().to_string();
    let (status, body) = call(
        app,
        "POST",
        "/api/channels",
        owner,
        Some(json!({
            "id": id,
            "channel_name": format!("fwd-ch-{}", Uuid::new_v4().simple()),
            "is_private": is_private,
            "encrypted": encrypted,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "create_channel failed: {body}");
    for member in members {
        let (status, body) = call(
            app,
            "POST",
            &format!("/api/channels/{id}/users"),
            owner,
            Some(json!({ "user_id": member })),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "join channel failed: {body}");
    }
    Ok(id)
}

async fn create_dm_channel(state: &AppState, u1: &str, u2: &str) -> anyhow::Result<String> {
    let id = Uuid::new_v4().to_string();
    let (a, b) = if u1 <= u2 { (u1, u2) } else { (u2, u1) };
    let name = format!("dm:{a}:{b}");
    sql_exec(
        state,
        "INSERT INTO channels(id, channel_name, created_user_id, is_private, is_dm) \
         VALUES($1, $2, $3, true, true) RETURNING json_build_object('ok', true)::text",
        &[&id, &name, u1],
    )
    .await?;
    for u in [u1, u2] {
        sql_exec(
            state,
            "INSERT INTO channel_user(channel_id, user_id, joined_at) \
             VALUES($1, $2, CURRENT_TIMESTAMP) RETURNING json_build_object('ok', true)::text",
            &[&id, u],
        )
        .await?;
    }
    Ok(id)
}

async fn post_plain_message(
    app: &Router,
    user: &str,
    channel_id: &str,
    content: &str,
    forwarded_ids: Option<Vec<&str>>,
) -> anyhow::Result<(StatusCode, Value)> {
    let id = Uuid::new_v4().to_string();
    let mut payload = json!({
        "id": id,
        "channel_id": channel_id,
        "content": content,
    });
    if let Some(fwds) = forwarded_ids {
        payload["forwarded_message_ids"] = json!(fwds);
    }
    call(app, "POST", "/api/messages", user, Some(payload)).await
}

fn sign_slack(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("v0={hex}")
}

#[cfg(unix)]
fn build_slack_zip(files: &[(&str, Value)]) -> anyhow::Result<Vec<u8>> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(buf);
    let options = SimpleFileOptions::default();
    for (name, content) in files {
        writer.start_file(*name, options)?;
        writer.write_all(&serde_json::to_vec(content)?)?;
    }
    Ok(writer.finish()?.into_inner())
}

#[tokio::test]
async fn forward_post_validation_idempotency_edit_and_delete() -> anyhow::Result<()> {
    let config = dev_config();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "fwd_alice").await?;
    let bob = create_user(&app, &state, "fwd_bob").await?;

    let public_src_ch = create_channel(&app, &alice, false, false, &[]).await?;
    let private_secret_ch = create_channel(&app, &bob, true, false, &[]).await?;
    let dest_ch = create_channel(&app, &alice, false, false, &[]).await?;
    let e2ee_ch = create_channel(&app, &bob, true, true, &[]).await?;

    // 転送元の公開投稿を作る
    let (status, src_msg) =
        post_plain_message(&app, &bob, &public_src_ch, "original post", None).await?;
    assert_eq!(status, StatusCode::OK, "{src_msg}");
    let src_id = src_msg["data"]["id"].as_str().unwrap().to_string();

    // 閲覧権限のない非公開チャンネルの投稿・削除済み投稿・E2EE 投稿
    let (status, priv_msg) =
        post_plain_message(&app, &bob, &private_secret_ch, "secret post", None).await?;
    assert_eq!(status, StatusCode::OK, "{priv_msg}");
    let priv_id = priv_msg["data"]["id"].as_str().unwrap().to_string();

    let (status, del_msg) =
        post_plain_message(&app, &bob, &public_src_ch, "to delete", None).await?;
    assert_eq!(status, StatusCode::OK, "{del_msg}");
    let del_id = del_msg["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/messages/{del_id}"),
        &bob,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let enc_src_id = Uuid::new_v4().to_string();
    sql_exec(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, content, ciphertext, epoch) \
         VALUES($1, $2, $3, '', '{\"ciphertext\":\"x\"}'::jsonb, 1) \
         RETURNING json_build_object('ok', true)::text",
        &[&enc_src_id, &bob, &e2ee_ch],
    )
    .await?;

    // 存在しない / 削除済み / 閲覧権限なし / E2EE 転送元は全て 404（存在オラクル防止）
    let missing_id = Uuid::new_v4().to_string();
    for bad_src in [&missing_id, &del_id, &priv_id, &enc_src_id] {
        let (status, body) =
            post_plain_message(&app, &alice, &dest_ch, "attempt", Some(vec![bad_src])).await?;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "bad_src={bad_src} body={body}"
        );
    }

    // 生配列長が上限（1 件）を超える場合は重複を含んでも 400、不正 UUID も 400
    assert_eq!(MAX_FORWARDED_MESSAGES_USER_POST, 1);
    let (status, body) = post_plain_message(
        &app,
        &alice,
        &dest_ch,
        "too many dup",
        Some(vec![src_id.as_str(), src_id.as_str()]),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, src2_msg) =
        post_plain_message(&app, &bob, &public_src_ch, "second post", None).await?;
    assert_eq!(status, StatusCode::OK);
    let src2_id = src2_msg["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = post_plain_message(
        &app,
        &alice,
        &dest_ch,
        "two distinct",
        Some(vec![src_id.as_str(), src2_id.as_str()]),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) =
        post_plain_message(&app, &alice, &dest_ch, "bad uuid", Some(vec!["not-a-uuid"])).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // コメントなし転送の作成 + 冪等性チェック（同一 ID & 同一 forwarded_message_ids -> 200、相違 -> 409）
    let idem_id = Uuid::new_v4().to_string();
    let (status, created) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({
            "id": idem_id,
            "channel_id": dest_ch,
            "content": "",
            "forwarded_message_ids": [format!(" {src_id} ")],
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["data"]["content"], "");
    assert_eq!(created["data"]["forwarded_message_ids"], json!([src_id]));

    let (status, replay) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({
            "id": idem_id,
            "channel_id": dest_ch,
            "content": "",
            "forwarded_message_ids": [src_id],
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["data"]["id"], idem_id);

    let (status, conflict) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({
            "id": idem_id,
            "channel_id": dest_ch,
            "content": "",
            "forwarded_message_ids": [src2_id],
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    // コメント付き編集 → 再度コメントなし（空文字）への編集が可能
    let (status, edited) = call(
        &app,
        "PATCH",
        &format!("/api/messages/{idem_id}"),
        &alice,
        Some(json!({ "content": "added comment" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["data"]["content"], "added comment");

    let (status, cleared) = call(
        &app,
        "PATCH",
        &format!("/api/messages/{idem_id}"),
        &alice,
        Some(json!({ "content": "" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["data"]["content"], "");

    // 通常メッセージは空編集不可
    let normal_id = Uuid::new_v4().to_string();
    let (status, _) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({ "id": normal_id, "channel_id": dest_ch, "content": "plain" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/api/messages/{normal_id}"),
        &alice,
        Some(json!({ "content": "" })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 論理削除時に DB トリガーで forwarded_message_ids も NULL になる
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/messages/{idem_id}"),
        &alice,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let row = sql_one(
        &state,
        "SELECT json_build_object('deleted', deleted_at IS NOT NULL, 'is_null', forwarded_message_ids IS NULL)::text \
         FROM messages WHERE id = $1",
        &[&idem_id],
    )
    .await?;
    assert_eq!(row["deleted"], json!(true));
    assert_eq!(row["is_null"], json!(true));

    Ok(())
}

#[tokio::test]
async fn resolve_forwards_enforces_visibility_no_store_and_oracle_safety() -> anyhow::Result<()> {
    let config = dev_config();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "res_alice").await?;
    let bob = create_user(&app, &state, "res_bob").await?;
    let guest = create_user(&app, &state, "res_guest").await?;
    let bot = create_user(&app, &state, "res_bot").await?;

    let pub_unjoined = create_channel(&app, &alice, false, false, &[]).await?;
    let pub_joined = create_channel(&app, &alice, false, false, &[&guest, &bot]).await?;
    sql_exec(
        &state,
        "UPDATE users SET is_guest = true WHERE id = $1 RETURNING json_build_object('ok', true)::text",
        &[&guest],
    )
    .await?;
    sql_exec(
        &state,
        "UPDATE users SET is_bot = true WHERE id = $1 RETURNING json_build_object('ok', true)::text",
        &[&bot],
    )
    .await?;
    let priv_ch = create_channel(&app, &alice, true, false, &[]).await?;
    let dm_ch = create_dm_channel(&state, &alice, &bob).await?;
    let e2ee_ch = create_channel(&app, &alice, true, true, &[]).await?;

    // 公開・添付あり（display_data->attachments 欠落時フォールバック確認用）・2段階転送・削除済み
    let (_, m_pub) = post_plain_message(&app, &alice, &pub_unjoined, "pub msg", None).await?;
    let id_pub = m_pub["data"]["id"].as_str().unwrap().to_string();

    // 添付レコードを紐付け、display_data から attachments をあえて除去する
    let att_id = Uuid::new_v4().to_string();
    sql_exec(
        &state,
        "INSERT INTO attachments(id, message_id, uploader_id, channel_id, file_name, file_size, content_type, storage_key) \
         VALUES($1, $2, $3, $4, 'notes.pdf', 128, 'application/pdf', 'k1') \
         RETURNING json_build_object('ok', true)::text",
        &[&att_id, &id_pub, &alice, &pub_unjoined],
    )
    .await?;
    sql_exec(
        &state,
        "UPDATE messages SET display_data = display_data - 'attachments' WHERE id = $1 \
         RETURNING json_build_object('ok', true)::text",
        &[&id_pub],
    )
    .await?;

    let (_, m_pub_joined) =
        post_plain_message(&app, &alice, &pub_joined, "joined pub", None).await?;
    let id_pub_joined = m_pub_joined["data"]["id"].as_str().unwrap().to_string();

    let (_, m_priv) = post_plain_message(&app, &alice, &priv_ch, "priv msg", None).await?;
    let id_priv = m_priv["data"]["id"].as_str().unwrap().to_string();

    let (_, m_priv_del) = post_plain_message(&app, &alice, &priv_ch, "priv del", None).await?;
    let id_priv_del = m_priv_del["data"]["id"].as_str().unwrap().to_string();
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/messages/{id_priv_del}"),
        &alice,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let (_, m_dm) = post_plain_message(&app, &alice, &dm_ch, "dm msg", None).await?;
    let id_dm = m_dm["data"]["id"].as_str().unwrap().to_string();

    // 転送を含むメッセージ自身を転送先に指定すると has_forwards == true になる
    let (status, m_outer_src) = post_plain_message(
        &app,
        &alice,
        &pub_unjoined,
        "has child",
        Some(vec![&id_pub]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{m_outer_src}");
    let id_has_forwards = m_outer_src["data"]["id"].as_str().unwrap().to_string();

    let id_e2ee = Uuid::new_v4().to_string();
    sql_exec(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, content, ciphertext, epoch) \
         VALUES($1, $2, $3, '', '{\"ciphertext\":\"c\"}'::jsonb, 1) \
         RETURNING json_build_object('ok', true)::text",
        &[&id_e2ee, &alice, &e2ee_ch],
    )
    .await?;
    let id_missing = Uuid::new_v4().to_string();

    let query_ids = json!({
        "ids": [&id_pub, &id_pub_joined, &id_priv, &id_priv_del, &id_dm, &id_has_forwards, &id_e2ee, &id_missing]
    });

    // メンバー（alice）: priv + priv_del(deleted) + dm(dm_participants あり) + attachments fallback + has_forwards
    let (status, headers, res_alice) = call_with_headers(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &alice,
        Some(query_ids.clone()),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{res_alice}");
    assert_eq!(
        headers.get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
    let items_a = &res_alice["data"]["items"];
    assert_eq!(items_a[&id_pub]["status"], "ok");
    assert_eq!(items_a[&id_pub]["attachments"][0]["id"], att_id);
    assert_eq!(items_a[&id_priv]["status"], "ok");
    assert_eq!(items_a[&id_priv_del]["status"], "deleted");
    assert_eq!(items_a[&id_dm]["status"], "ok");
    assert_eq!(
        items_a[&id_dm]["dm_participants"].as_array().unwrap().len(),
        2
    );
    assert_eq!(items_a[&id_has_forwards]["has_forwards"], json!(true));
    assert_eq!(items_a[&id_e2ee], json!({ "status": "unavailable" }));
    assert_eq!(items_a[&id_missing], json!({ "status": "unavailable" }));

    // 非メンバー（bob）: pub + dm は ok、priv & priv_del は共に {"status":"unavailable"} 完全一致（削除オラクル防止）
    let (status, res_bob) = call(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &bob,
        Some(query_ids.clone()),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{res_bob}");
    let items_b = &res_bob["data"]["items"];
    assert_eq!(items_b[&id_pub]["status"], "ok");
    assert_eq!(items_b[&id_dm]["status"], "ok");
    assert_eq!(items_b[&id_priv], json!({ "status": "unavailable" }));
    assert_eq!(items_b[&id_priv_del], json!({ "status": "unavailable" }));

    // guest / bot: 未参加の公開チャンネルは {"status":"unavailable"} 完全一致、参加済み公開チャンネルのみ ok
    for viewer in [&guest, &bot] {
        let (status, res) = call(
            &app,
            "POST",
            "/api/messages/forwards/resolve",
            viewer,
            Some(json!({ "ids": [&id_pub, &id_pub_joined] })),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{res}");
        assert_eq!(
            res["data"]["items"][&id_pub],
            json!({ "status": "unavailable" })
        );
        assert_eq!(res["data"]["items"][&id_pub_joined]["status"], "ok");
    }

    // 上限超え（101 件）は 400
    let too_many: Vec<String> = (0..=MAX_RESOLVE_FORWARD_IDS)
        .map(|_| Uuid::new_v4().to_string())
        .collect();
    let (status, _) = call(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &alice,
        Some(json!({ "ids": too_many })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn e2ee_rejects_forwarded_message_ids_in_api_and_db_triggers() -> anyhow::Result<()> {
    let config = dev_config();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "e2ee_alice").await?;
    let plain_ch = create_channel(&app, &alice, false, false, &[]).await?;
    let e2ee_ch = create_channel(&app, &alice, true, true, &[]).await?;

    let (_, m_src) = post_plain_message(&app, &alice, &plain_ch, "src", None).await?;
    let src_id = m_src["data"]["id"].as_str().unwrap().to_string();

    // API レベルで暗号化チャンネルへの転送投稿を拒否（E2EE_PLAINTEXT_FORBIDDEN）
    let (status, body) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({
            "id": Uuid::new_v4().to_string(),
            "channel_id": e2ee_ch,
            "content": "",
            "epoch": 1,
            "forwarded_message_ids": [src_id],
            "ciphertext": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "sender_key": "k",
                "device_id": "d",
                "session_id": "s",
                "ciphertext": "c",
            },
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "E2EE_PLAINTEXT_FORBIDDEN", "{body}");

    // DB トリガー（直接 INSERT & 事後暗号化行 UPDATE）も SQLSTATE 23514 で拒否する
    let direct_id = Uuid::new_v4().to_string();
    let fwd_json = json!([src_id]).to_string();
    let err = sql_raw_err(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, content, ciphertext, epoch, forwarded_message_ids) \
         VALUES($1, $2, $3, '', '{\"ciphertext\":\"c\"}'::jsonb, 1, $4::jsonb)",
        &[&direct_id, &alice, &e2ee_ch, &fwd_json],
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("plaintext") || err.contains("23514"),
        "unexpected error: {err}"
    );

    let pre_e2ee_ch = create_channel(&app, &alice, true, false, &[]).await?;
    let (_, pre_msg) = post_plain_message(&app, &alice, &pre_e2ee_ch, "before e2ee", None).await?;
    let pre_msg_id = pre_msg["data"]["id"].as_str().unwrap().to_string();
    sql_exec(
        &state,
        "UPDATE channels SET encrypted = true WHERE id = $1 RETURNING json_build_object('ok', true)::text",
        &[&pre_e2ee_ch],
    )
    .await?;
    let err = sql_raw_err(
        &state,
        "UPDATE messages SET forwarded_message_ids = $2::jsonb WHERE id = $1",
        &[&pre_msg_id, &fwd_json],
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("before the channel was encrypted") || err.contains("23514"),
        "unexpected error: {err}"
    );

    Ok(())
}

#[tokio::test]
async fn notification_display_trigger_and_context_track_has_forwards() -> anyhow::Result<()> {
    let config = dev_config();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "notif_alice").await?;
    let bob = create_user(&app, &state, "notif_bob").await?;
    let ch = create_channel(&app, &alice, false, false, &[&bob]).await?;

    let (_, src) = post_plain_message(&app, &alice, &ch, "src", None).await?;
    let src_id = src["data"]["id"].as_str().unwrap().to_string();

    // 転送あり投稿への通知は初期作成時点で has_forwards == true
    let (_, fwd_msg) =
        post_plain_message(&app, &alice, &ch, "", Some(vec![src_id.as_str()])).await?;
    let fwd_id = fwd_msg["data"]["id"].as_str().unwrap().to_string();
    let notif_fwd = Uuid::new_v4().to_string();
    sql_exec(
        &state,
        "INSERT INTO notifications(id, user_id, actor_id, channel_id, message_id, notification_type) \
         VALUES($1, $2, $3, $4, $5, 'mention') RETURNING json_build_object('ok', true)::text",
        &[&notif_fwd, &bob, &alice, &ch, &fwd_id],
    )
    .await?;
    let (status, ctx_fwd) = call(
        &app,
        "GET",
        &format!("/api/notifications/{notif_fwd}/context"),
        &bob,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{ctx_fwd}");
    assert_eq!(ctx_fwd["message"]["has_forwards"], json!(true));

    // 通常投稿では has_forwards を含めない。後から forwarded_message_ids を UPDATE 補完すると
    // message_notifications_display トリガーで has_forwards が true に同期される
    let (_, plain_msg) = post_plain_message(&app, &alice, &ch, "backfill target", None).await?;
    let plain_id = plain_msg["data"]["id"].as_str().unwrap().to_string();
    let notif_plain = Uuid::new_v4().to_string();
    sql_exec(
        &state,
        "INSERT INTO notifications(id, user_id, actor_id, channel_id, message_id, notification_type) \
         VALUES($1, $2, $3, $4, $5, 'mention') RETURNING json_build_object('ok', true)::text",
        &[&notif_plain, &bob, &alice, &ch, &plain_id],
    )
    .await?;
    let (status, ctx_before) = call(
        &app,
        "GET",
        &format!("/api/notifications/{notif_plain}/context"),
        &bob,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{ctx_before}");
    assert!(ctx_before["message"].get("has_forwards").is_none());
    let display_before = sql_one(
        &state,
        "SELECT display_data::text FROM notification_events WHERE id = $1",
        &[&notif_plain],
    )
    .await?;
    assert_eq!(
        display_before["message"]["has_forwards"],
        json!(false),
        "{display_before}"
    );

    let fwd_json = json!([src_id]).to_string();
    sql_exec(
        &state,
        "UPDATE messages SET forwarded_message_ids = $2::jsonb WHERE id = $1 \
         RETURNING json_build_object('ok', true)::text",
        &[&plain_id, &fwd_json],
    )
    .await?;
    let display_after = sql_one(
        &state,
        "SELECT display_data::text FROM notification_events WHERE id = $1",
        &[&notif_plain],
    )
    .await?;
    assert_eq!(
        display_after["message"]["has_forwards"],
        json!(true),
        "trigger did not sync notification_events.display_data: {display_after}"
    );
    let (status, ctx_after) = call(
        &app,
        "GET",
        &format!("/api/notifications/{notif_plain}/context"),
        &bob,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{ctx_after}");
    assert_eq!(ctx_after["message"]["has_forwards"], json!(true));

    Ok(())
}

#[tokio::test]
async fn slack_hints_and_backfill_forwards_restore_forwards_without_bumping_edited_state()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let key = [7u8; 32];
    let team = format!("TFWD{}", &Uuid::new_v4().simple().to_string()[..8]);
    let signing_secret = "test-signing-secret";
    let slack = MockServer::start().await;

    let mut config = dev_config();
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();
    config.slack.signing_secret = Some(signing_secret.into());
    config.slack.app_id = None;

    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config.clone());
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "slack_alice").await?;
    let enc = slack::crypto::encrypt(&key, "xoxp-alice")?;
    sql_exec(
        &state,
        "INSERT INTO slack_user_links(user_id, team_id, slack_user_id, access_token_enc) \
         VALUES($1, $2, 'UA', $3) RETURNING json_build_object('ok', true)::text",
        &[&alice, &team, &enc],
    )
    .await?;

    let slack_ch = format!("CFWD{}", &team[4..]);
    let zoo_ch = create_channel(&app, &alice, false, false, &[]).await?;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let ts_src = format!("{}.000100", now_secs - 500);
    let ts_root = format!("{}.000200", now_secs - 400);
    let ts_reply = format!("{}.000250", now_secs - 350);
    let ts_commentless = format!("{}.000300", now_secs - 300);
    let ts_hint = format!("{}.000400", now_secs - 200);
    let cursor = format!("{}.000000", now_secs - 600);

    sql_exec(
        &state,
        "INSERT INTO slack_channels(team_id, slack_channel_id, channel_id, last_ts) \
         VALUES($1, $2, $3, $4) RETURNING json_build_object('ok', true)::text",
        &[&team, &slack_ch, &zoo_ch, &cursor],
    )
    .await?;
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::enqueue_team(&mut tx, &team).await?;
        tx.commit().await?;
    }

    // 既存の転送元・ルート投稿・返信投稿・hints 対象投稿を事前作成（forwarded_message_ids は NULL、updated_at = NULL）
    let id_src = slack::message_id(&team, &slack_ch, &ts_src).to_string();
    let id_root = slack::message_id(&team, &slack_ch, &ts_root).to_string();
    let id_reply = slack::message_id(&team, &slack_ch, &ts_reply).to_string();
    let id_hint = slack::message_id(&team, &slack_ch, &ts_hint).to_string();
    let id_commentless = slack::message_id(&team, &slack_ch, &ts_commentless).to_string();

    for (msg_id, parent, body) in [
        (&id_src, None, "slack original"),
        (&id_root, None, "existing root forward"),
        (&id_reply, Some(id_root.as_str()), "existing reply forward"),
        (&id_hint, None, "hint target body"),
    ] {
        sql_exec(
            &state,
            "INSERT INTO messages(id, user_id, channel_id, parent_id, content, revision) \
             VALUES($1, $2, $3, NULLIF($4, ''), $5, 1) \
             RETURNING json_build_object('ok', true)::text",
            &[msg_id, &alice, &zoo_ch, parent.unwrap_or(""), body],
        )
        .await?;
    }
    sql_exec(
        &state,
        "INSERT INTO slack_message_map(team_id, slack_channel_id, ts, message_id, origin) \
         VALUES($1, $2, $3, $4, 'slack'), ($1, $2, $5, $6, 'slack'), ($1, $2, $7, $8, 'slack'), ($1, $2, $9, $10, 'slack') \
         RETURNING json_build_object('ok', true)::text",
        &[
            &team, &slack_ch, &ts_src, &id_src, &ts_root, &id_root, &ts_reply, &id_reply, &ts_hint, &id_hint,
        ],
    )
    .await?;

    let forward_attachment = json!([{
        "is_share": true,
        "channel_id": slack_ch,
        "ts": ts_src,
        "from_url": format!("https://example.slack.com/archives/{slack_ch}/p{}", ts_src.replace('.', "")),
        "text": "slack original",
    }]);

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", &slack_ch))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": { "id": slack_ch, "is_private": false },
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": { "id": "UA", "name": "alice", "real_name": "Alice", "profile": { "display_name": "alice" } },
        })))
        .mount(&slack)
        .await;

    // 1. backfill_forwards::run のモック: 履歴にルート(コメント付き既存)・コメントなし新規・thread_root を返す
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &slack_ch))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {
                    "type": "message",
                    "ts": ts_commentless,
                    "user": "UA",
                    "text": "",
                    "attachments": forward_attachment,
                },
                {
                    "type": "message",
                    "ts": ts_root,
                    "user": "UA",
                    "text": "existing root forward",
                    "thread_ts": ts_root,
                    "reply_count": 1,
                    "attachments": forward_attachment,
                },
                {
                    "type": "message",
                    "ts": ts_src,
                    "user": "UA",
                    "text": "slack original",
                },
            ],
            "has_more": false,
        })))
        .with_priority(5)
        .mount(&slack)
        .await;

    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("channel", &slack_ch))
        .and(query_param("ts", &ts_root))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {
                    "type": "message",
                    "ts": ts_root,
                    "user": "UA",
                    "text": "existing root forward",
                    "thread_ts": ts_root,
                    "attachments": forward_attachment,
                },
                {
                    "type": "message",
                    "ts": ts_reply,
                    "user": "UA",
                    "text": "existing reply forward",
                    "thread_ts": ts_root,
                    "attachments": forward_attachment,
                },
            ],
            "has_more": false,
        })))
        .mount(&slack)
        .await;

    // dry-run: 件数のみ集計し DB は更新しない
    let dry = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: false,
            channel: Some(slack_ch.clone()),
            oldest: None,
            max_pages: None,
        },
    )
    .await?;
    assert_eq!(dry.channels, 1);
    assert_eq!(dry.matched_backfills, 2, "{dry:?}"); // root + reply
    assert_eq!(dry.matched_inserts, 1, "{dry:?}");
    assert_eq!(dry.applied_backfills, 0);
    assert_eq!(dry.applied_inserts, 0);

    // apply=true: root と reply の forwarded_message_ids を補完（updated_at/revision 不変）、コメントなし転送を新規 INSERT
    let applied = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: true,
            channel: Some(slack_ch.clone()),
            oldest: None,
            max_pages: None,
        },
    )
    .await?;
    assert_eq!(applied.applied_backfills, 2, "{applied:?}");
    assert_eq!(applied.applied_inserts, 1, "{applied:?}");

    for id in [&id_root, &id_reply] {
        let row = sql_one(
            &state,
            "SELECT json_build_object(\
                'fwd', forwarded_message_ids, \
                'updated_null', updated_at IS NULL, \
                'rev', revision\
             )::text FROM messages WHERE id = $1",
            &[id],
        )
        .await?;
        assert_eq!(row["fwd"], json!([id_src]), "id={id} row={row}");
        assert_eq!(
            row["updated_null"],
            json!(true),
            "updated_at bumped on {id}"
        );
        assert_eq!(row["rev"], json!(1), "revision bumped on {id}");
    }
    let inserted = sql_one(
        &state,
        "SELECT json_build_object('content', content, 'fwd', forwarded_message_ids)::text \
         FROM messages WHERE id = $1",
        &[&id_commentless],
    )
    .await?;
    assert_eq!(inserted["content"], "");
    assert_eq!(inserted["fwd"], json!([id_src]));

    // 2 回目 apply=true は冪等（applied_* == 0）
    let second = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: true,
            channel: Some(slack_ch.clone()),
            oldest: None,
            max_pages: None,
        },
    )
    .await?;
    assert_eq!(second.applied_backfills, 0, "{second:?}");
    assert_eq!(second.applied_inserts, 0, "{second:?}");

    // 2. hints::edit 経路: forwarded_message_ids が欠落している既存投稿への message_changed で
    // updated_at / revision を変えずに forwarded_message_ids のみ補完する
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &slack_ch))
        .and(query_param("latest", &ts_hint))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [{
                "type": "message",
                "ts": ts_hint,
                "user": "UA",
                "text": "hint target body",
                "attachments": forward_attachment,
            }],
            "has_more": false,
        })))
        .with_priority(1)
        .mount(&slack)
        .await;

    let event_body = serde_json::to_vec(&json!({
        "type": "event_callback",
        "team_id": team,
        "api_app_id": "A1",
        "event": {
            "type": "message",
            "subtype": "message_changed",
            "channel": slack_ch,
            "message": { "ts": ts_hint, "user": "UA", "text": "hint target body" },
        }
    }))?;
    let sig_ts = now_secs.to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/slack/events")
                .header("content-type", "application/json")
                .header("x-slack-request-timestamp", &sig_ts)
                .header(
                    "x-slack-signature",
                    sign_slack(signing_secret, &sig_ts, &event_body),
                )
                .body(Body::from(event_body))?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    sql_exec(
        &state,
        "UPDATE slack_channel_wakes SET first_at = now() - interval '10 seconds' \
         WHERE team_id = $1 RETURNING json_build_object('ok', true)::text",
        &[&team],
    )
    .await?;
    sql_exec(
        &state,
        "UPDATE slack_channel_syncs SET available_at = now() + interval '1 hour', \
         last_synced_at = now() - interval '1 hour', rechecked_at = now(), \
         not_before = NULL, lease_token = NULL, lease_until = NULL, running = false \
         WHERE team_id = $1 RETURNING json_build_object('ok', true)::text",
        &[&team],
    )
    .await?;
    for _ in 0..20 {
        if !sync::run_once(&state).await? {
            break;
        }
    }
    let hint_row = sql_one(
        &state,
        "SELECT json_build_object('fwd', forwarded_message_ids, 'updated_null', updated_at IS NULL, 'rev', revision)::text \
         FROM messages WHERE id = $1",
        &[&id_hint],
    )
    .await?;
    assert_eq!(hint_row["fwd"], json!([id_src]), "{hint_row}");
    assert_eq!(hint_row["updated_null"], json!(true), "{hint_row}");
    assert_eq!(hint_row["rev"], json!(1), "{hint_row}");

    Ok(())
}

/// Windows・Linux 共通で `sync::run_once` → `worker::insert_message` の既存メッセージ再取込バックフィル
/// （親メッセージ・スレッド返信とも `updated_at` / `revision` 不変）とコメントなし転送 INSERT を検証する。
#[tokio::test]
async fn slack_sync_exercises_worker_insert_message_backfill_and_commentless_forward()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let key = [8u8; 32];
    let team = format!("TSYW{}", &Uuid::new_v4().simple().to_string()[..8]);
    let slack = MockServer::start().await;

    let mut config = dev_config();
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "syw_alice").await?;
    let enc = slack::crypto::encrypt(&key, "xoxp-alice")?;
    sql_exec(
        &state,
        "INSERT INTO slack_user_links(user_id, team_id, slack_user_id, access_token_enc) \
         VALUES($1, $2, 'UA', $3) RETURNING json_build_object('ok', true)::text",
        &[&alice, &team, &enc],
    )
    .await?;

    let slack_ch = format!("CSYW{}", &team[4..]);
    let zoo_ch = create_channel(&app, &alice, false, false, &[]).await?;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let cursor = format!("{}.000000", now_secs - 600);
    let ts_src = format!("{}.000100", now_secs - 500);
    let ts_root = format!("{}.000200", now_secs - 400);
    let ts_reply = format!("{}.000250", now_secs - 350);
    let ts_commentless = format!("{}.000300", now_secs - 300);

    sql_exec(
        &state,
        "INSERT INTO slack_channels(team_id, slack_channel_id, channel_id, last_ts) \
         VALUES($1, $2, $3, $4) RETURNING json_build_object('ok', true)::text",
        &[&team, &slack_ch, &zoo_ch, &cursor],
    )
    .await?;

    let id_src = slack::message_id(&team, &slack_ch, &ts_src).to_string();
    let id_root = slack::message_id(&team, &slack_ch, &ts_root).to_string();
    let id_reply = slack::message_id(&team, &slack_ch, &ts_reply).to_string();
    let id_commentless = slack::message_id(&team, &slack_ch, &ts_commentless).to_string();

    for (msg_id, parent, body) in [
        (&id_src, None, "slack src"),
        (&id_root, None, "root missing fwd"),
        (&id_reply, Some(id_root.as_str()), "reply missing fwd"),
    ] {
        sql_exec(
            &state,
            "INSERT INTO messages(id, user_id, channel_id, parent_id, content, revision) \
             VALUES($1, $2, $3, NULLIF($4, ''), $5, 1) \
             RETURNING json_build_object('ok', true)::text",
            &[msg_id, &alice, &zoo_ch, parent.unwrap_or(""), body],
        )
        .await?;
    }
    sql_exec(
        &state,
        "INSERT INTO slack_message_map(team_id, slack_channel_id, ts, message_id, origin) \
         VALUES($1, $2, $3, $4, 'slack'), ($1, $2, $5, $6, 'slack'), ($1, $2, $7, $8, 'slack') \
         RETURNING json_build_object('ok', true)::text",
        &[
            &team, &slack_ch, &ts_src, &id_src, &ts_root, &id_root, &ts_reply, &id_reply,
        ],
    )
    .await?;

    let fwd_att = json!([{
        "is_share": true,
        "channel_id": slack_ch,
        "ts": ts_src,
        "from_url": format!("https://example.slack.com/archives/{slack_ch}/p{}", ts_src.replace('.', "")),
        "text": "slack src",
    }]);

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", &slack_ch))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": { "id": slack_ch, "is_private": false },
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": { "id": "UA", "name": "alice", "real_name": "Alice", "profile": { "display_name": "alice" } },
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &slack_ch))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {
                    "type": "message",
                    "ts": ts_commentless,
                    "user": "UA",
                    "text": "",
                    "attachments": fwd_att,
                },
                {
                    "type": "message",
                    "ts": ts_root,
                    "user": "UA",
                    "text": "root missing fwd",
                    "thread_ts": ts_root,
                    "reply_count": 1,
                    "latest_reply": ts_reply,
                    "attachments": fwd_att,
                },
            ],
            "has_more": false,
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("channel", &slack_ch))
        .and(query_param("ts", &ts_root))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {
                    "type": "message",
                    "ts": ts_root,
                    "user": "UA",
                    "text": "root missing fwd",
                    "thread_ts": ts_root,
                    "attachments": fwd_att,
                },
                {
                    "type": "message",
                    "ts": ts_reply,
                    "user": "UA",
                    "text": "reply missing fwd",
                    "thread_ts": ts_root,
                    "attachments": fwd_att,
                },
            ],
            "has_more": false,
        })))
        .with_priority(1)
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "ok": true, "messages": [], "has_more": false })),
        )
        .with_priority(9)
        .mount(&slack)
        .await;

    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::enqueue_team(&mut tx, &team).await?;
        tx.commit().await?;
    }
    sql_exec(
        &state,
        "UPDATE slack_channel_syncs SET available_at = now() - interval '1 second', \
         last_synced_at = now() - interval '1 hour', not_before = NULL, \
         lease_token = NULL, lease_until = NULL, running = false \
         WHERE team_id = $1 RETURNING json_build_object('ok', true)::text",
        &[&team],
    )
    .await?;

    let mut ran_steps = 0usize;
    for _ in 0..20 {
        if !sync::run_once(&state).await? {
            break;
        }
        ran_steps += 1;
    }
    let sync_diag = sql_one(
        &state,
        "SELECT json_build_object(\
            'ran_steps', $2::text, \
            'last_ts', sc.last_ts, \
            'attempts', s.attempts, \
            'last_error', s.last_error, \
            'running', s.running, \
            'commentless_exists', EXISTS(SELECT 1 FROM messages WHERE id = $3)\
         )::text \
         FROM slack_channels sc LEFT JOIN slack_channel_syncs s ON s.team_id = sc.team_id AND s.slack_channel_id = sc.slack_channel_id \
         WHERE sc.team_id = $1",
        &[&team, &ran_steps.to_string(), &id_commentless],
    )
    .await?;

    for id in [&id_root, &id_reply] {
        let row = sql_one(
            &state,
            "SELECT json_build_object('fwd', forwarded_message_ids, 'updated_null', updated_at IS NULL, 'rev', revision)::text \
             FROM messages WHERE id = $1",
            &[id],
        )
        .await?;
        assert_eq!(
            row["fwd"],
            json!([id_src]),
            "id={id} row={row} sync={sync_diag}"
        );
        assert_eq!(row["updated_null"], json!(true), "id={id}");
        assert_eq!(row["rev"], json!(1), "id={id}");
    }
    let inserted = sql_one(
        &state,
        "SELECT json_build_object('content', content, 'fwd', forwarded_message_ids)::text \
         FROM messages WHERE id = $1",
        &[&id_commentless],
    )
    .await?;
    assert_eq!(inserted["content"], "");
    assert_eq!(inserted["fwd"], json!([id_src]));

    Ok(())
}

/// `TempZip` は Linux で advisory `flock` を使い、Windows では mandatory lock により
/// `ExportArchive::open` が `os error 33` になるため Linux CI 上で S3 mock 経路を検証する。
#[cfg(unix)]
#[tokio::test]
async fn slack_zip_reimport_backfills_forwarded_ids_via_mock_s3() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let s3_mock = MockServer::start().await;

    let mut config = dev_config();
    config.s3.endpoint = s3_mock.uri();
    config.s3.force_path_style = true;
    config.s3.bucket = "test-bucket".into();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "zip_alice").await?;
    sql_exec(
        &state,
        "INSERT INTO role(id, role_name) VALUES('role_admin', 'admin') ON CONFLICT DO NOTHING \
         RETURNING json_build_object('ok', true)::text",
        &[],
    )
    .await?;
    sql_exec(
        &state,
        "INSERT INTO user_role(user_id, role_id) VALUES($1, 'role_admin') ON CONFLICT DO NOTHING \
         RETURNING json_build_object('ok', true)::text",
        &[&alice],
    )
    .await?;
    let zoo_ch = create_channel(&app, &alice, false, false, &[]).await?;
    let team = format!("TZIP{}", &Uuid::new_v4().simple().to_string()[..8]);

    let ts_src = "1704067200.000100";
    let ts_fwd = "1704067260.000200";
    let ts_reply = "1704067290.000250";
    let ts_commentless = "1704067320.000300";
    let id_src = slack::message_id(&team, "CZ1", ts_src).to_string();
    let id_fwd = slack::message_id(&team, "CZ1", ts_fwd).to_string();
    let id_reply = slack::message_id(&team, "CZ1", ts_reply).to_string();
    let id_commentless = slack::message_id(&team, "CZ1", ts_commentless).to_string();

    // 既存取り込みメッセージ（forwarded_message_ids なし、updated_at = NULL、親投稿＋スレッド返信、
    // 作成日時は Slack の ts と一致させ worker::insert_message の derived 判定を通す）
    sql_exec(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, parent_id, content, created_at, revision) \
         VALUES\
         ($1, $2, $3, NULL, 'existing src', (to_timestamp($6::numeric) AT TIME ZONE 'UTC')::timestamp(3), 1), \
         ($4, $2, $3, NULL, 'with fwd', (to_timestamp($7::numeric) AT TIME ZONE 'UTC')::timestamp(3), 1), \
         ($5, $2, $3, $4, 'reply fwd', (to_timestamp($8::numeric) AT TIME ZONE 'UTC')::timestamp(3), 1) \
         RETURNING json_build_object('ok', true)::text",
        &[
            &id_src, &alice, &zoo_ch, &id_fwd, &id_reply, ts_src, ts_fwd, ts_reply,
        ],
    )
    .await?;

    let zip_bytes = build_slack_zip(&[
        (
            "channels.json",
            json!([{ "id": "CZ1", "name": "general", "is_archived": false }]),
        ),
        (
            "users.json",
            json!([{ "id": "UA", "team_id": team, "name": "zip_alice", "profile": { "display_name": "zip_alice" } }]),
        ),
        (
            "general/2024-01-01.json",
            json!([
                {
                    "type": "message",
                    "ts": ts_src,
                    "user": "UA",
                    "text": "existing src",
                },
                {
                    "type": "message",
                    "ts": ts_fwd,
                    "user": "UA",
                    "text": "with fwd",
                    "thread_ts": ts_fwd,
                    "attachments": [{
                        "is_share": true,
                        "channel_id": "CZ1",
                        "ts": ts_src,
                        "text": "existing src",
                    }],
                },
                {
                    "type": "message",
                    "ts": ts_reply,
                    "user": "UA",
                    "text": "reply fwd",
                    "thread_ts": ts_fwd,
                    "attachments": [{
                        "is_share": true,
                        "channel_id": "CZ1",
                        "ts": ts_src,
                        "text": "existing src",
                    }],
                },
                {
                    "type": "message",
                    "ts": ts_commentless,
                    "user": "UA",
                    "text": "",
                    "attachments": [{
                        "is_msg_unfurl": true,
                        "channel_id": "CZ1",
                        "ts": ts_src,
                        "text": "existing src",
                    }],
                },
            ]),
        ),
    ])?;

    let storage_key = format!("slack-imports/{}.zip", Uuid::new_v4());
    Mock::given(method("GET"))
        .and(path(format!("/test-bucket/{storage_key}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/zip")
                .insert_header("etag", "\"mock-etag\"")
                .set_body_bytes(zip_bytes),
        )
        .mount(&s3_mock)
        .await;

    let src_job = Uuid::new_v4().to_string();
    let run_job = Uuid::new_v4().to_string();
    let mappings = json!([{
        "slack_channel_id": "CZ1",
        "action": "existing",
        "channel_id": zoo_ch,
    }])
    .to_string();
    sql_exec(
        &state,
        "INSERT INTO slack_import_jobs(id, created_by, status, file_name, file_size, storage_key, team_id) \
         VALUES($1, $2, 'ready', 'reimport.zip', 100, $3, $4) \
         RETURNING json_build_object('ok', true)::text",
        &[&src_job, &alice, &storage_key, &team],
    )
    .await?;
    sql_exec(
        &state,
        "INSERT INTO slack_import_jobs(id, created_by, status, file_name, file_size, storage_key, team_id, source_id, mappings) \
         VALUES($1, $2, 'importing', 'reimport.zip', 100, $3, $4, $5, $6) \
         RETURNING json_build_object('ok', true)::text",
        &[&run_job, &alice, &storage_key, &team, &src_job, &mappings],
    )
    .await?;

    for _ in 0..30 {
        if !worker::run_once(&state).await? {
            let status_row = sql_one(
                &state,
                "SELECT json_build_object('status', status, 'error', error)::text FROM slack_import_jobs WHERE id = $1",
                &[&run_job],
            )
            .await?;
            if status_row["status"] == "completed" {
                break;
            }
            if status_row["status"] == "failed" {
                panic!("worker failed: {status_row}");
            }
        }
    }

    for id in [&id_fwd, &id_reply] {
        let fwd_row = sql_one(
            &state,
            "SELECT json_build_object('fwd', forwarded_message_ids, 'updated_null', updated_at IS NULL, 'rev', revision)::text \
             FROM messages WHERE id = $1",
            &[id],
        )
        .await?;
        assert_eq!(fwd_row["fwd"], json!([id_src]), "{fwd_row}");
        assert_eq!(fwd_row["updated_null"], json!(true), "{fwd_row}");
        assert_eq!(fwd_row["rev"], json!(1), "{fwd_row}");
    }

    let commentless_row = sql_one(
        &state,
        "SELECT json_build_object('content', content, 'fwd', forwarded_message_ids)::text \
         FROM messages WHERE id = $1",
        &[&id_commentless],
    )
    .await?;
    assert_eq!(commentless_row["content"], "");
    assert_eq!(commentless_row["fwd"], json!([id_src]));

    Ok(())
}

/// 大文字・別表記の UUID で作成したメッセージを、元の表記および正規化小文字・ハイフンなし表記で転送作成・冪等再送・一括解決できることを検証する。
#[tokio::test]
async fn uppercase_message_id_forwards_and_resolves_consistently() -> anyhow::Result<()> {
    let mut config = dev_config();
    config.slack.signing_secret = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "up_alice").await?;
    let ch = create_channel(&app, &alice, false, false, &[]).await?;
    let src_uuid = Uuid::new_v4();
    let upper_src_id = src_uuid.hyphenated().to_string().to_ascii_uppercase();
    let lower_src_id = src_uuid.hyphenated().to_string();
    let simple_src_id = src_uuid.simple().to_string().to_ascii_uppercase();
    let braced_src_id = src_uuid.braced().to_string().to_ascii_uppercase();

    let (status, created) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({
            "id": upper_src_id,
            "channel_id": ch,
            "content": "uppercase source",
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["data"]["id"], upper_src_id);

    // 元の表記そのままでの転送
    let (status, fwd) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(json!({
            "channel_id": ch,
            "content": "fwd upper",
            "forwarded_message_ids": [upper_src_id],
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{fwd}");
    assert_eq!(fwd["data"]["forwarded_message_ids"], json!([upper_src_id]));

    // 別表記（小文字・波括弧など）を指定しても転送元を解決して作成でき、同じリクエストでの冪等再送も通る
    let client_fwd_id = Uuid::new_v4().to_string();
    let lower_payload = json!({
        "id": client_fwd_id,
        "channel_id": ch,
        "content": "fwd via canonical lower",
        "forwarded_message_ids": [lower_src_id],
    });
    let (status, fwd_lower) = call(
        &app,
        "POST",
        "/api/messages",
        &alice,
        Some(lower_payload.clone()),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{fwd_lower}");
    assert_eq!(
        fwd_lower["data"]["forwarded_message_ids"],
        json!([upper_src_id])
    );

    let (status, fwd_retry) =
        call(&app, "POST", "/api/messages", &alice, Some(lower_payload)).await?;
    assert_eq!(status, StatusCode::OK, "{fwd_retry}");
    assert_eq!(fwd_retry["data"]["id"], client_fwd_id);

    let (status, resolved) = call(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &alice,
        Some(json!({ "ids": [upper_src_id, lower_src_id, simple_src_id, braced_src_id] })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    for key in [&upper_src_id, &lower_src_id, &simple_src_id, &braced_src_id] {
        assert_eq!(resolved["data"]["items"][key]["status"], "ok", "key={key}");
        assert_eq!(
            resolved["data"]["items"][key]["content"], "uppercase source",
            "key={key}"
        );
    }

    Ok(())
}

/// Slack の未取り込み転送元 ID が事前作成でなりすまされないこと、および stale な bot hint で
/// 既存の `forwarded_message_ids` が消えないことを検証する。
#[tokio::test]
async fn slack_forward_reservation_blocks_spoofing_and_stale_bot_hint_preserves_forwards()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let key = [11u8; 32];
    let team = format!("TRSV{}", &Uuid::new_v4().simple().to_string()[..8]);
    let signing_secret = "reserve-signing-secret";
    let slack = MockServer::start().await;

    let mut config = dev_config();
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();
    config.slack.signing_secret = Some(signing_secret.into());

    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config.clone());
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "rsv_alice").await?;
    let attacker = create_user(&app, &state, "rsv_attacker").await?;
    let enc = slack::crypto::encrypt(&key, "xoxp-alice")?;
    sql_exec(
        &state,
        "INSERT INTO slack_user_links(user_id, team_id, slack_user_id, access_token_enc) \
         VALUES($1, $2, 'UA', $3) RETURNING json_build_object('ok', true)::text",
        &[&alice, &team, &enc],
    )
    .await?;

    let slack_dest = format!("CDST{}", &team[4..]);
    let slack_src_reserved = format!("CRSV{}", &team[4..]);
    let slack_src_preoccupied = format!("CPRC{}", &team[4..]);
    let dest_ch = create_channel(&app, &alice, false, false, &[&attacker]).await?;
    let atk_ch = create_channel(&app, &attacker, false, false, &[]).await?;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let ts_reserved = "1704067201.000100";
    let ts_preoccupied = "1704067202.000100";
    let ts_fwd_rsv = format!("{}.000200", now_secs - 200);
    let ts_fwd_pre = format!("{}.000250", now_secs - 190);
    let ts_bot_hint = format!("{}.000300", now_secs - 180);
    let cursor = format!("{}.000000", now_secs - 500);

    let reserved_id = slack::message_id(&team, &slack_src_reserved, ts_reserved).to_string();
    let preoccupied_id =
        slack::message_id(&team, &slack_src_preoccupied, ts_preoccupied).to_string();
    let id_fwd_rsv = slack::message_id(&team, &slack_dest, &ts_fwd_rsv).to_string();
    let id_fwd_pre = slack::message_id(&team, &slack_dest, &ts_fwd_pre).to_string();
    let id_bot_hint = slack::message_id(&team, &slack_dest, &ts_bot_hint).to_string();

    // 1) Slack 取り込み前に攻撃者が決定的 ID で別投稿を作成済み -> backfill はその偽装 ID を転送先に結び付けない
    let (status, spoof_pre) = call(
        &app,
        "POST",
        "/api/messages",
        &attacker,
        Some(json!({
            "id": preoccupied_id,
            "channel_id": atk_ch,
            "content": "spoofed before import",
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{spoof_pre}");

    // 既存の bot 投稿（forwarded_message_ids 保持済み、edited なし）も用意
    sql_exec(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, content, created_at, revision, forwarded_message_ids) \
         VALUES($1, $2, $3, 'bot body', (to_timestamp($4::numeric) AT TIME ZONE 'UTC')::timestamp(3), 1, $5::jsonb) \
         RETURNING json_build_object('ok', true)::text",
        &[&id_bot_hint, &alice, &dest_ch, &ts_bot_hint, &json!([reserved_id]).to_string()],
    )
    .await?;
    sql_exec(
        &state,
        "INSERT INTO slack_channels(team_id, slack_channel_id, channel_id, last_ts) \
         VALUES($1, $2, $3, $4) RETURNING json_build_object('ok', true)::text",
        &[&team, &slack_dest, &dest_ch, &cursor],
    )
    .await?;

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", &slack_dest))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": { "id": slack_dest, "is_private": false },
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": { "id": "UA", "name": "alice", "real_name": "Alice", "profile": { "display_name": "alice" } },
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &slack_dest))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {
                    "type": "message",
                    "ts": ts_fwd_rsv,
                    "user": "UA",
                    "text": "fwd to unimported",
                    "attachments": [{
                        "is_share": true,
                        "channel_id": slack_src_reserved,
                        "ts": ts_reserved,
                        "from_url": format!("https://example.slack.com/archives/{slack_src_reserved}/p{}", ts_reserved.replace('.', "")),
                        "text": "secret slack msg",
                    }],
                },
                {
                    "type": "message",
                    "ts": ts_fwd_pre,
                    "user": "UA",
                    "text": "fwd to preoccupied",
                    "attachments": [{
                        "is_share": true,
                        "channel_id": slack_src_preoccupied,
                        "ts": ts_preoccupied,
                        "from_url": format!("https://example.slack.com/archives/{slack_src_preoccupied}/p{}", ts_preoccupied.replace('.', "")),
                        "text": "preoccupied slack msg",
                    }],
                },
            ],
            "has_more": false,
        })))
        .with_priority(5)
        .mount(&slack)
        .await;

    let report = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: true,
            channel: Some(slack_dest.clone()),
            oldest: None,
            max_pages: None,
        },
    )
    .await?;
    assert_eq!(report.applied_inserts, 1, "{report:?}");

    // 先に占有されていた偽装 ID 向け転送は forwarded_message_ids が付与されない
    let pre_row = sql_one(
        &state,
        "SELECT json_build_object('exists', count(*) > 0, 'fwd', max(forwarded_message_ids::text))::text \
         FROM messages WHERE id = $1",
        &[&id_fwd_pre],
    )
    .await?;
    assert_eq!(pre_row["fwd"], Value::Null, "{pre_row}");

    // 未取り込みの reserved_id は slack_forward_targets に予約され、小文字・大文字とも POST /api/messages は 409 Conflict
    let rsv_row = sql_one(
        &state,
        "SELECT json_build_object('fwd', forwarded_message_ids)::text FROM messages WHERE id = $1",
        &[&id_fwd_rsv],
    )
    .await?;
    assert_eq!(rsv_row["fwd"], json!([reserved_id]));

    for attempt_id in [
        reserved_id.clone(),
        reserved_id.to_ascii_uppercase(),
        reserved_id.replace('-', "").to_ascii_uppercase(),
        format!("{{{}}}", reserved_id.to_ascii_uppercase()),
    ] {
        let (status, err_body) = call(
            &app,
            "POST",
            "/api/messages",
            &attacker,
            Some(json!({
                "id": attempt_id,
                "channel_id": atk_ch,
                "content": "spoof after reservation",
            })),
        )
        .await?;
        assert_eq!(status, StatusCode::CONFLICT, "{err_body}");
    }

    // 予約済み・未取り込みの ID は解決 API で unavailable
    let (status, res_before) = call(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &alice,
        Some(json!({ "ids": [reserved_id] })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{res_before}");
    assert_eq!(
        res_before["data"]["items"][&reserved_id],
        json!({ "status": "unavailable" })
    );

    // 対象チャンネル取り込み後に正当な Slack メッセージが入れば解決可能になり、別チャンネルへ直接書かれた不正行は拒否される
    let genuine_ch = create_channel(&app, &alice, false, false, &[]).await?;
    sql_exec(
        &state,
        "INSERT INTO slack_channels(team_id, slack_channel_id, channel_id, last_ts) \
         VALUES($1, $2, $3, $4) RETURNING json_build_object('ok', true)::text",
        &[&team, &slack_src_reserved, &genuine_ch, &cursor],
    )
    .await?;
    // まず誤った channel / 時刻で作られた行を想定してチェック
    sql_exec(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, content, created_at, revision) \
         VALUES($1, $2, $3, 'wrong channel spoof', (to_timestamp($4::numeric) AT TIME ZONE 'UTC')::timestamp(3), 1) \
         RETURNING json_build_object('ok', true)::text",
        &[&reserved_id, &attacker, &atk_ch, ts_reserved],
    )
    .await?;
    let (status, res_wrong_ch) = call(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &alice,
        Some(json!({ "ids": [reserved_id] })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{res_wrong_ch}");
    assert_eq!(
        res_wrong_ch["data"]["items"][&reserved_id],
        json!({ "status": "unavailable" })
    );

    sql_exec(
        &state,
        "UPDATE messages SET channel_id = $2, content = 'genuine slack src' WHERE id = $1 \
         RETURNING json_build_object('ok', true)::text",
        &[&reserved_id, &genuine_ch],
    )
    .await?;
    let (status, res_after) = call(
        &app,
        "POST",
        "/api/messages/forwards/resolve",
        &alice,
        Some(json!({ "ids": [reserved_id] })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{res_after}");
    assert_eq!(res_after["data"]["items"][&reserved_id]["status"], "ok");
    assert_eq!(
        res_after["data"]["items"][&reserved_id]["content"],
        "genuine slack src"
    );

    // 2) bot 投稿（edited なし）に対する stale hint（attachments 欠落応答）で既存 forwarded_message_ids が消えないこと
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::enqueue_team(&mut tx, &team).await?;
        tx.commit().await?;
    }
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &slack_dest))
        .and(query_param("latest", &ts_bot_hint))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [{
                "type": "message",
                "ts": ts_bot_hint,
                "bot_id": "B1",
                "text": "bot body",
            }],
            "has_more": false,
        })))
        .with_priority(1)
        .mount(&slack)
        .await;
    let bot_event = serde_json::to_vec(&json!({
        "type": "event_callback",
        "team_id": team,
        "api_app_id": "A1",
        "event": {
            "type": "message",
            "subtype": "message_changed",
            "channel": slack_dest,
            "message": { "ts": ts_bot_hint, "bot_id": "B1", "text": "bot body" },
        }
    }))?;
    let sig_ts = now_secs.to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/slack/events")
                .header("content-type", "application/json")
                .header("x-slack-request-timestamp", &sig_ts)
                .header(
                    "x-slack-signature",
                    sign_slack(signing_secret, &sig_ts, &bot_event),
                )
                .body(Body::from(bot_event))?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    sql_exec(
        &state,
        "UPDATE slack_channel_wakes SET first_at = now() - interval '10 seconds' \
         WHERE team_id = $1 RETURNING json_build_object('ok', true)::text",
        &[&team],
    )
    .await?;
    sql_exec(
        &state,
        "UPDATE slack_channel_syncs SET available_at = now() + interval '1 hour', \
         last_synced_at = now() - interval '1 hour', rechecked_at = now(), \
         not_before = NULL, lease_token = NULL, lease_until = NULL, running = false \
         WHERE team_id = $1 RETURNING json_build_object('ok', true)::text",
        &[&team],
    )
    .await?;
    for _ in 0..20 {
        if !sync::run_once(&state).await? {
            break;
        }
    }
    let bot_row = sql_one(
        &state,
        "SELECT json_build_object('fwd', forwarded_message_ids, 'updated_null', updated_at IS NULL, 'rev', revision)::text \
         FROM messages WHERE id = $1",
        &[&id_bot_hint],
    )
    .await?;
    assert_eq!(bot_row["fwd"], json!([reserved_id]), "{bot_row}");
    assert_eq!(bot_row["updated_null"], json!(true), "{bot_row}");
    assert_eq!(bot_row["rev"], json!(1), "{bot_row}");

    Ok(())
}

/// バックフィルの異常系（空 `--channel`・本番 HTTP 設定・`conversations.info` 一時失敗・`Retry-After > 300s`）と
/// ページネーション／`--oldest` 境界（`next_cursor` のみ返る次ページ・古い親のスレッド内新着返信）を検証する。
#[tokio::test]
async fn slack_backfill_handles_validation_transient_errors_cursors_oldest_and_long_retry()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let key = [13u8; 32];
    let team = format!("TEDG{}", &Uuid::new_v4().simple().to_string()[..8]);
    let slack = MockServer::start().await;

    let mut config = dev_config();
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();

    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config.clone());
    let app = create_router(state.clone());

    let alice = create_user(&app, &state, "edge_alice").await?;
    let enc = slack::crypto::encrypt(&key, "xoxp-alice")?;
    sql_exec(
        &state,
        "INSERT INTO slack_user_links(user_id, team_id, slack_user_id, access_token_enc) \
         VALUES($1, $2, 'UA', $3) RETURNING json_build_object('ok', true)::text",
        &[&alice, &team, &enc],
    )
    .await?;

    // 1) 空文字 `--channel` と本番 HTTP `SLACK_API_BASE_URL` の拒否
    let empty_err = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: true,
            channel: Some("   ".into()),
            oldest: None,
            max_pages: None,
        },
    )
    .await
    .unwrap_err();
    assert!(empty_err.to_string().contains("--channel"), "{empty_err}");

    let mut prod_cfg = config.clone();
    prod_cfg.environment = "production".into();
    prod_cfg.slack.api_base_url = "http://slack.example.com/api".into();
    let http_err =
        backfill_forwards::run(&state.db, &prod_cfg, &backfill_forwards::Options::default())
            .await
            .unwrap_err();
    assert!(
        http_err.to_string().contains("SLACK_API_BASE_URL"),
        "{http_err}"
    );

    // 2) `next_cursor` のみ（`has_more` 省略）のページング＋ `--oldest` より古い親のスレッド内新着返信を回収
    let ch_ok = format!("COK{}", &team[4..]);
    let ch_err = format!("CERR{}", &team[4..]);
    let ch_rl = format!("CRL{}", &team[4..]);
    let zoo_ok = create_channel(&app, &alice, false, false, &[]).await?;
    let zoo_err = create_channel(&app, &alice, false, false, &[]).await?;
    let zoo_rl = create_channel(&app, &alice, false, false, &[]).await?;

    for (sc, zc) in [(&ch_ok, &zoo_ok), (&ch_err, &zoo_err), (&ch_rl, &zoo_rl)] {
        sql_exec(
            &state,
            "INSERT INTO slack_channels(team_id, slack_channel_id, channel_id, last_ts) \
             VALUES($1, $2, $3, '1704067000.000000') RETURNING json_build_object('ok', true)::text",
            &[&team, sc, zc],
        )
        .await?;
    }

    let ts_src = "1704067100.000100";
    let ts_old_root = "1704067150.000100";
    let ts_oldest = "1704067200.000000";
    let ts_old_ignored = "1704067180.000100";
    let ts_page2_msg = "1704067250.000100";
    let ts_reply_in_window = "1704067260.000200";
    let id_src = slack::message_id(&team, &ch_ok, ts_src).to_string();
    let id_old_ignored = slack::message_id(&team, &ch_ok, ts_old_ignored).to_string();
    let id_page2 = slack::message_id(&team, &ch_ok, ts_page2_msg).to_string();
    let id_reply_win = slack::message_id(&team, &ch_ok, ts_reply_in_window).to_string();

    sql_exec(
        &state,
        "INSERT INTO messages(id, user_id, channel_id, content, created_at, revision) \
         VALUES($1, $2, $3, 'src', (to_timestamp($4::numeric) AT TIME ZONE 'UTC')::timestamp(3), 1) \
         RETURNING json_build_object('ok', true)::text",
        &[&id_src, &alice, &zoo_ok, ts_src],
    )
    .await?;

    let att = json!([{
        "is_share": true,
        "channel_id": ch_ok,
        "ts": ts_src,
        "from_url": format!("https://example.slack.com/archives/{ch_ok}/p{}", ts_src.replace('.', "")),
        "text": "src",
    }]);

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", &ch_ok))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": { "id": ch_ok, "is_private": false },
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": { "id": "UA", "name": "alice", "real_name": "Alice", "profile": { "display_name": "alice" } },
        })))
        .mount(&slack)
        .await;
    // history page 1: has_more 省略 + response_metadata.next_cursor のみ + oldest より古い投稿と古い親（latest_reply >= oldest）
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &ch_ok))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {
                    "type": "message",
                    "ts": ts_old_ignored,
                    "user": "UA",
                    "text": "",
                    "attachments": att,
                },
                {
                    "type": "message",
                    "ts": ts_old_root,
                    "user": "UA",
                    "text": "old root",
                    "thread_ts": ts_old_root,
                    "reply_count": 1,
                    "latest_reply": ts_reply_in_window,
                },
            ],
            "response_metadata": { "next_cursor": "cur_page_2" },
        })))
        .with_priority(5)
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", &ch_ok))
        .and(query_param("cursor", "cur_page_2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [{
                "type": "message",
                "ts": ts_page2_msg,
                "user": "UA",
                "text": "",
                "attachments": att,
            }],
        })))
        .with_priority(1)
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("channel", &ch_ok))
        .and(query_param("ts", ts_old_root))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                { "type": "message", "ts": ts_old_root, "user": "UA", "text": "old root", "thread_ts": ts_old_root },
                { "type": "message", "ts": ts_reply_in_window, "user": "UA", "text": "", "thread_ts": ts_old_root, "attachments": att },
            ],
        })))
        .mount(&slack)
        .await;

    let win_report = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: true,
            channel: Some(ch_ok.clone()),
            oldest: Some(ts_oldest.into()),
            max_pages: None,
        },
    )
    .await?;
    assert_eq!(win_report.applied_inserts, 2, "{win_report:?}");
    let exists = sql_one(
        &state,
        "SELECT json_build_object(\
            'old', EXISTS(SELECT 1 FROM messages WHERE id = $1), \
            'p2', EXISTS(SELECT 1 FROM messages WHERE id = $2 AND forwarded_message_ids IS NOT NULL), \
            'reply', EXISTS(SELECT 1 FROM messages WHERE id = $3 AND forwarded_message_ids IS NOT NULL)\
         )::text",
        &[&id_old_ignored, &id_page2, &id_reply_win],
    )
    .await?;
    assert_eq!(exists["old"], json!(false), "{exists}");
    assert_eq!(exists["p2"], json!(true), "{exists}");
    assert_eq!(exists["reply"], json!(true), "{exists}");

    // 3) conversations.info の一時エラー（HTTP 500）は伝播し、長い Retry-After (>300s) は待たずにエラー報告する
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", &ch_err))
        .respond_with(ResponseTemplate::new(500))
        .mount(&slack)
        .await;
    let info_err = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: false,
            channel: Some(ch_err.clone()),
            oldest: None,
            max_pages: None,
        },
    )
    .await
    .unwrap_err();
    assert!(info_err.to_string().contains(&ch_err), "{info_err}");

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", &ch_rl))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "600"))
        .mount(&slack)
        .await;
    let started = std::time::Instant::now();
    let rl_err = backfill_forwards::run(
        &state.db,
        &config,
        &backfill_forwards::Options {
            apply: false,
            channel: Some(ch_rl.clone()),
            oldest: None,
            max_pages: None,
        },
    )
    .await
    .unwrap_err();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "should not sleep on Retry-After: 600"
    );
    let rl_msg = rl_err.to_string();
    assert!(rl_msg.contains("retry_after=600s"), "{rl_msg}");
    assert!(rl_msg.contains("resume_after="), "{rl_msg}");

    Ok(())
}

/// App API (`POST /api/apps/v1/messages`) は `forwarded_message_ids` を未サポートとして 400 で拒否する。
#[tokio::test]
async fn app_api_rejects_forwarded_message_ids() -> anyhow::Result<()> {
    let mut config = dev_config();
    config.slack.signing_secret = None;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let owner = create_user(&app, &state, "app_owner").await?;
    let admin = create_user(&app, &state, "app_admin").await?;
    sql_exec(
        &state,
        "INSERT INTO role(id, role_name) VALUES('role_admin', 'admin') ON CONFLICT DO NOTHING \
         RETURNING json_build_object('ok', true)::text",
        &[],
    )
    .await?;
    sql_exec(
        &state,
        "INSERT INTO user_role(user_id, role_id) VALUES($1, 'role_admin') \
         ON CONFLICT DO NOTHING RETURNING json_build_object('ok', true)::text",
        &[&admin],
    )
    .await?;

    let (status, created_app) = call(
        &app,
        "POST",
        "/api/apps",
        &owner,
        Some(json!({
            "name": "Forward Guard App",
            "description": "Rejects forwarded_message_ids",
            "manifest": {
                "scopes": ["messages:read", "messages:write"],
                "events": [],
                "transport": "websocket",
                "dm_enabled": false,
                "redirect_uris": ["https://example.com/callback"]
            }
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{created_app}");
    let app_id = created_app["id"].as_str().unwrap();

    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/apps/{app_id}/actions/submit"),
        &owner,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/apps/{app_id}/actions/install"),
        &admin,
        Some(json!({ "revision": 1 })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (status, token_body) = call(
        &app,
        "POST",
        &format!("/api/apps/{app_id}/actions/token"),
        &owner,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{token_body}");
    let token = token_body["access_token"].as_str().unwrap();

    let ch = create_channel(&app, &owner, true, false, &[]).await?;
    let (status, join_res) = call(
        &app,
        "POST",
        &format!("/api/apps/{app_id}/channels/{ch}"),
        &owner,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{join_res}");

    let (status, src_post) = call(
        &app,
        "POST",
        "/api/messages",
        &owner,
        Some(json!({ "channel_id": ch, "content": "source for app test" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{src_post}");
    let src_id = src_post["data"]["id"].as_str().unwrap();

    let (status, ok_post) = call(
        &app,
        "POST",
        "/api/apps/v1/messages",
        token,
        Some(json!({ "channel_id": ch, "content": "normal app message" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{ok_post}");

    let (status, rejected) = call(
        &app,
        "POST",
        "/api/apps/v1/messages",
        token,
        Some(json!({
            "channel_id": ch,
            "content": "app forward attempt",
            "forwarded_message_ids": [src_id]
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");

    Ok(())
}
