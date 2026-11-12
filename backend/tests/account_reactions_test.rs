use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn account_reactions_can_be_added_and_removed_without_a_workspace_registry_entry()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let app = create_router(AppState::new(init_db(&config.database_url).await?, config));
    let user = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let message = uuid::Uuid::new_v4().to_string();
    async fn send(
        app: &axum::Router,
        method: &str,
        path: &str,
        user: &str,
        body: Value,
    ) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("X-Dev-User", user)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    assert_eq!(send(&app, "POST", "/api/users", &user, json!({"id":user,"user_name":format!("account_{}",user.replace('-',"")),"display_name":"Fixture"})).await, StatusCode::OK);
    assert_eq!(send(&app, "POST", "/api/channels", &user, json!({"id":channel,"channel_name":format!("account-{}",uuid::Uuid::new_v4().simple()),"is_private":false})).await, StatusCode::OK);
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/messages",
            &user,
            json!({"id":message,"channel_id":channel,"content":"Fixture"})
        )
        .await,
        StatusCode::OK
    );
    let path = format!("/api/messages/{message}/reactions");
    assert_eq!(
        send(&app, "POST", &path, &user, json!({"emoji":":@alice:"})).await,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "DELETE",
            &format!("{path}?emoji=%3A%40alice%3A"),
            &user,
            json!(null)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "POST",
            &path,
            &user,
            json!({"emoji":":missing_registry_fixture:"})
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&app, "POST", &path, &user, json!({"emoji":":@@alice:"})).await,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}
