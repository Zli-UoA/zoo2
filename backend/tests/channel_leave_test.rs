//! 自分でチャンネルを退出した日時（`channel_leave`）の結合テスト。
//!
//! 自分の退出だけを記録し、キックでは記録しない。再退出では上書きする。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Env {
    app: Router,
    db: toasty::Db,
}

async fn env() -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await.unwrap();
    let app = create_router(AppState::new(db.clone(), config));
    Env { app, db }
}

impl Env {
    async fn call(&self, method: &str, path: &str, actor: &str, body: Value) -> StatusCode {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("X-Dev-User", actor)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let _ = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        status
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let payload = json!({
            "id": id,
            "user_name": format!("leave_{}", Uuid::new_v4().simple()),
            "display_name": "leave",
        });
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        id
    }

    /// `members` を足した公開チャンネルを作る（作成者は `creator`）。
    async fn channel(&self, creator: &str, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        let status = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("leave-{}", Uuid::new_v4().simple()), "is_private": false}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        for member in members {
            let status = self
                .call(
                    "POST",
                    &format!("/api/channels/{id}/users"),
                    creator,
                    json!({"user_id": member}),
                )
                .await;
            assert_eq!(status, StatusCode::OK);
        }
        id
    }

    async fn leave(&self, channel: &str, user: &str) -> StatusCode {
        self.call(
            "POST",
            &format!("/api/channels/{channel}/leave"),
            user,
            Value::Null,
        )
        .await
    }

    /// 退出日（ミリ秒の epoch）。記録がなければ None。
    async fn left_at_millis(&self, user: &str, channel: &str) -> Option<i64> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let rows = backend::apps::query(
            &mut tx,
            "SELECT (extract(epoch FROM left_at) * 1000)::bigint::text FROM channel_leave WHERE user_id = $1 AND channel_id = $2",
            &[user, channel],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        rows.first().map(|v| v.as_i64().unwrap())
    }
}

#[tokio::test]
async fn leaving_records_the_leave_date_for_that_user_only() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let channel = env.channel(&alice, &[&bob, &carol]).await;

    assert_eq!(env.left_at_millis(&bob, &channel).await, None);
    let before = jiff::Timestamp::now().as_millisecond();
    assert_eq!(env.leave(&channel, &bob).await, StatusCode::OK);
    let after = jiff::Timestamp::now().as_millisecond();

    let left_at = env.left_at_millis(&bob, &channel).await.unwrap();
    assert!(
        (before - 1000..=after + 1000).contains(&left_at),
        "left_at={left_at} before={before} after={after}"
    );
    // 他のメンバーの行は作らない。
    assert_eq!(env.left_at_millis(&carol, &channel).await, None);
    assert_eq!(env.left_at_millis(&alice, &channel).await, None);
}

#[tokio::test]
async fn leaving_again_after_rejoining_overwrites_the_leave_date() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.channel(&alice, &[&bob]).await;

    assert_eq!(env.leave(&channel, &bob).await, StatusCode::OK);
    let first = env.left_at_millis(&bob, &channel).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(
        env.call(
            "POST",
            &format!("/api/channels/{channel}/users"),
            &alice,
            json!({"user_id": bob}),
        )
        .await,
        StatusCode::OK
    );
    // 再参加しても、前回の退出日は残る。
    assert_eq!(env.left_at_millis(&bob, &channel).await, Some(first));

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(env.leave(&channel, &bob).await, StatusCode::OK);
    let second = env.left_at_millis(&bob, &channel).await.unwrap();
    assert!(second > first, "second={second} first={first}");
}

#[tokio::test]
async fn kick_and_rejected_leave_do_not_record_a_leave_date() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let channel = env.channel(&alice, &[&bob]).await;

    // キックでは記録しない（自分で退出したわけではない）。
    assert_eq!(
        env.call(
            "DELETE",
            &format!("/api/channels/{channel}/users/{bob}"),
            &alice,
            Value::Null,
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(env.left_at_millis(&bob, &channel).await, None);

    // 作成者は退出できず、メンバーでない人の退出は 404。どちらも記録しない。
    assert_eq!(env.leave(&channel, &alice).await, StatusCode::BAD_REQUEST);
    assert_eq!(env.leave(&channel, &carol).await, StatusCode::NOT_FOUND);
    assert_eq!(env.left_at_millis(&alice, &channel).await, None);
    assert_eq!(env.left_at_millis(&carol, &channel).await, None);
}
