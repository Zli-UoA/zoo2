use super::{handle_web_push_response, send_current_web_push};
use crate::{
    config::Config,
    db::init_db,
    models::{PushSubscription, User, now_utc_millis},
};
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

async fn create_users(db: &mut dyn toasty::db::Executor) -> anyhow::Result<[String; 2]> {
    let users = [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()];
    for id in &users {
        User::create()
            .id(id.as_str())
            .user_name(id.as_str())
            .display_name("通知テスト")
            .is_bot(false)
            .created_at(now_utc_millis())
            .exec(db)
            .await?;
    }
    Ok(users)
}

async fn register(
    db: &mut dyn toasty::db::Executor,
    user: &str,
    id: &str,
    endpoint: &str,
    created_at: jiff::civil::DateTime,
) -> anyhow::Result<PushSubscription> {
    Ok(PushSubscription::create()
        .id(id)
        .user_id(user)
        .endpoint(endpoint)
        .p256dh("公開鍵")
        .auth_secret("認証用の鍵")
        .created_at(created_at)
        .exec(db)
        .await?)
}

#[tokio::test]
#[ignore = "マイグレーションを適用したPostgreSQLを使用"]
async fn web_push_only_sends_to_the_current_owner_and_keys() -> anyhow::Result<()> {
    let mut db = init_db(&Config::from_env().database_url).await?;
    let mut tx = db.transaction().await?;
    let [owner, next_owner] = create_users(&mut tx).await?;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .expect(2)
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let client = reqwest::Client::new();
    let send = || async { Ok(Some(client.post(&endpoint).send().await?)) };
    let id = Uuid::new_v4().to_string();
    let created_at = now_utc_millis();
    let original = register(&mut tx, &owner, &id, &endpoint, created_at).await?;

    for (p256dh, auth_secret) in [
        ("更新後の公開鍵", "認証用の鍵"),
        ("公開鍵", "更新後の認証用の鍵"),
    ] {
        PushSubscription::update_by_id(id.as_str())
            .p256dh(p256dh)
            .auth_secret(auth_secret)
            .exec(&mut tx)
            .await?;
        assert!(
            send_current_web_push(&mut tx, &original, send())
                .await?
                .is_none()
        );
    }
    PushSubscription::filter(PushSubscription::fields().id().eq(id.as_str()))
        .delete()
        .exec(&mut tx)
        .await?;
    assert!(
        send_current_web_push(&mut tx, &original, send())
            .await?
            .is_none()
    );

    let replacement = register(&mut tx, &next_owner, &id, &endpoint, created_at).await?;
    assert!(
        send_current_web_push(&mut tx, &original, send())
            .await?
            .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());

    let response = send_current_web_push(&mut tx, &replacement, send())
        .await?
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    handle_web_push_response(&mut tx, &replacement, response).await?;

    PushSubscription::update_by_id(id.as_str())
        .p256dh(None::<String>)
        .auth_secret(None::<String>)
        .exec(&mut tx)
        .await?;
    let legacy = PushSubscription::get_by_id(&mut tx, id.as_str()).await?;
    let response = send_current_web_push(&mut tx, &legacy, send())
        .await?
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "マイグレーションを適用したPostgreSQLを使用"]
async fn gone_response_only_removes_the_registration_that_was_sent() -> anyhow::Result<()> {
    let mut db = init_db(&Config::from_env().database_url).await?;
    let mut tx = db.transaction().await?;
    let [owner, next_owner] = create_users(&mut tx).await?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(410))
        .expect(3)
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let client = reqwest::Client::new();
    let send = || client.post(&endpoint).send();
    let id = Uuid::new_v4().to_string();
    let created_at = now_utc_millis();
    let original = register(&mut tx, &owner, &id, &endpoint, created_at).await?;

    let response = send().await?;
    PushSubscription::filter(PushSubscription::fields().id().eq(id.as_str()))
        .delete()
        .exec(&mut tx)
        .await?;
    let replacement = register(&mut tx, &next_owner, &id, &endpoint, created_at).await?;
    handle_web_push_response(&mut tx, &original, response).await?;
    let saved = PushSubscription::get_by_id(&mut tx, id.as_str()).await?;
    assert_eq!(saved.user_id, next_owner);

    let response = send().await?;
    let renewed_at = created_at.checked_add(jiff::Span::new().seconds(1))?;
    PushSubscription::update_by_id(id.as_str())
        .created_at(renewed_at)
        .exec(&mut tx)
        .await?;
    handle_web_push_response(&mut tx, &replacement, response).await?;
    let renewed = PushSubscription::get_by_id(&mut tx, id.as_str()).await?;
    assert_eq!(renewed.created_at, renewed_at);

    handle_web_push_response(&mut tx, &renewed, send().await?).await?;
    assert!(
        PushSubscription::filter(PushSubscription::fields().id().eq(id.as_str()))
            .first()
            .exec(&mut tx)
            .await?
            .is_none()
    );
    tx.rollback().await?;
    Ok(())
}
