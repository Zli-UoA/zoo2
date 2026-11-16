//! 論理削除したメッセージは、本文と付随データを DB に残さない（墓石の行だけが残る）。
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config,
    db::init_db,
    models::{Channel, Message, User, now_utc_millis},
    routes::create_router,
    state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn sql(db: &mut toasty::Db, query: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut tx = db.transaction().await?;
    let rows = backend::apps::query(&mut tx, query, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn delete_message(app: &Router, user: &str, id: &str) -> anyhow::Result<StatusCode> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/messages/{id}"))
                .header("X-Dev-User", user)
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(status)
}

async fn tombstone(db: &mut toasty::Db, id: &str) -> anyhow::Result<Value> {
    let rows = sql(
        db,
        "SELECT json_build_object('content',content,'card',card,'metadata',metadata,\
         'app_blocks',app_blocks,'app_attachments',app_attachments,'app_unfurls',app_unfurls,\
         'display_data',display_data,'deleted',deleted_at IS NOT NULL)::text FROM messages WHERE id=$1",
        &[id],
    )
    .await?;
    assert_eq!(rows.len(), 1, "the tombstone row must stay");
    Ok(rows[0].clone())
}

fn assert_cleared(row: &Value) {
    assert_eq!(row["deleted"], true);
    assert_eq!(row["content"], "");
    for column in [
        "card",
        "metadata",
        "app_blocks",
        "app_attachments",
        "app_unfurls",
    ] {
        assert!(row[column].is_null(), "{column} must be cleared: {row}");
    }
    assert_eq!(row["display_data"], json!({}), "{row}");
}

#[tokio::test]
async fn deleted_messages_keep_no_body_or_projection() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let owner = Uuid::new_v4().to_string();
    let now = now_utc_millis();
    User::create()
        .id(owner.as_str())
        .user_name(format!("purge_{owner}"))
        .display_name("Purge user")
        .is_bot(false)
        .created_at(now)
        .exec(&mut db)
        .await?;
    let channel = Uuid::new_v4().to_string();
    Channel::create()
        .id(channel.as_str())
        .channel_name(format!("purge-{channel}"))
        .created_user_id(owner.as_str())
        .is_private(false)
        .is_nsfw(false)
        .is_dm(false)
        .created_at(now)
        .exec(&mut db)
        .await?;

    let mut ids = Vec::new();
    for body in ["first body", "second body", "kept body"] {
        let id = Uuid::new_v4().to_string();
        Message::create()
            .id(id.as_str())
            .channel_id(channel.as_str())
            .user_id(owner.as_str())
            .content(body)
            .apub(false)
            .created_at(now)
            .exec(&mut db)
            .await?;
        sql(
            &mut db,
            "UPDATE messages SET card='card body', app_blocks='[{\"text\":\"block\"}]', \
             app_unfurls='[]', app_attachments='[{\"title\":\"a\"}]', \
             metadata='{\"event_type\":\"x\",\"event_payload\":{}}' WHERE id=$1",
            &[&id],
        )
        .await?;
        ids.push(id);
    }

    // 消去を確かめる列は、削除前に値が入っている。
    for id in &ids {
        let row = tombstone(&mut db, id).await?;
        for column in [
            "card",
            "metadata",
            "app_blocks",
            "app_attachments",
            "app_unfurls",
        ] {
            assert!(
                !row[column].is_null(),
                "{column} must be set before delete: {row}"
            );
        }
    }

    // DELETE /api/messages/{id}
    assert_eq!(delete_message(&app, &owner, &ids[0]).await?, StatusCode::OK);
    assert_cleared(&tombstone(&mut db, &ids[0]).await?);

    // 履歴の消去は、同じ deleted_at の更新だけで本文を消す。
    sql(
        &mut db,
        "UPDATE messages SET deleted_at=now(), updated_at=now() WHERE id=$1",
        &[&ids[1]],
    )
    .await?;
    assert_cleared(&tombstone(&mut db, &ids[1]).await?);

    // 削除後のリアクションなどで、投影が復活しない。
    sql(
        &mut db,
        "INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES ($1,$2,$3,$4)",
        &[&Uuid::new_v4().to_string(), &ids[0], &owner, "👍"],
    )
    .await?;
    assert_cleared(&tombstone(&mut db, &ids[0]).await?);

    // 削除していない行は変わらない。
    let kept = tombstone(&mut db, &ids[2]).await?;
    assert_eq!(kept["deleted"], false);
    assert_eq!(kept["content"], "kept body");
    assert_eq!(kept["card"], "card body");
    assert!(kept["display_data"]["author"].is_object(), "{kept}");
    Ok(())
}
