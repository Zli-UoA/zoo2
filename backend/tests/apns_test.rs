use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use backend::{
    config::Config, db::init_db, models::ApnsSubscription, routes::create_router, state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn native_registration_enforces_ownership_and_capability_cleanup() -> anyhow::Result<()> {
    // 別プロセスに使い捨ての署名鍵を渡し、開発者やCIのAPNs設定を使わずに両環境を検証する。
    const TEST_ENV: &str = "ZOO_APNS_REGISTRATION_TEST_CHILD";
    if std::env::var_os(TEST_ENV).is_none() {
        use openssl::{
            ec::{EcGroup, EcKey},
            nid::Nid,
            pkey::PKey,
        };
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
        let key = PKey::from_ec_key(EcKey::generate(&group)?)?;
        let status = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "native_registration_enforces_ownership_and_capability_cleanup",
                "--nocapture",
            ])
            .env(TEST_ENV, "1")
            .env("APPLE_TEAM_ID", "ABCDEFGHIJ")
            .env("APNS_SANDBOX_KEY_ID", "KLMNOPQRST")
            .env(
                "APNS_SANDBOX_PRIVATE_KEY",
                String::from_utf8(key.private_key_to_pem_pkcs8()?)?,
            )
            .env_remove("APNS_PRODUCTION_KEY_ID")
            .env_remove("APNS_PRODUCTION_PRIVATE_KEY")
            .status()?;
        anyhow::ensure!(status.success(), "端末通知の登録・解除テストに失敗しました");
        return Ok(());
    }
    tokio::runtime::Runtime::new()?.block_on(check_registration())
}

const REGISTER: &str = "/api/push/apns/subscriptions";
const REVOKE: &str = "/api/push/apns/revoke";
const CONFIG: &str = "/api/push/apns/config";
const TEST: &str = "/api/push/apns/test";

async fn post(
    app: &Router,
    uri: &str,
    user: Option<&str>,
    body: Value,
) -> anyhow::Result<Response> {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(user) = user {
        request = request.header("X-Dev-User", user);
    }
    Ok(app
        .clone()
        .oneshot(request.body(Body::from(body.to_string()))?)
        .await?)
}

fn registration(token: &str, secret: &str) -> Value {
    json!({"device_token":token, "environment":"sandbox", "revocation_secret":secret,
           "created_at":jiff::Timestamp::now().as_second()})
}

async fn check_registration() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let mut secured_config = config.clone();
    secured_config.dev_auth_enabled = false;
    let secured = create_router(AppState::new(db.clone(), secured_config));
    let app = create_router(AppState::new(db.clone(), config));
    let alice = Uuid::new_v4().to_string();
    let bob = Uuid::new_v4().to_string();
    for user in [&alice, &bob] {
        let body =
            json!({"id":user,"user_name":format!("apns_{user}"),"display_name":"通知テスト"});
        assert_eq!(
            post(&app, "/api/users", None, body).await?.status(),
            StatusCode::OK
        );
    }
    let get = |user: Option<&str>| {
        let mut request = Request::builder().uri(CONFIG);
        if let Some(user) = user {
            request = request.header("X-Dev-User", user);
        }
        request.body(Body::empty()).unwrap()
    };
    assert_eq!(
        secured.clone().oneshot(get(None)).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&secured, REGISTER, None, json!({})).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    let availability = app.clone().oneshot(get(Some(&alice))).await?;
    assert_eq!(availability.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(availability.into_body(), 4096).await?)?;
    assert_eq!(body, json!({"sandbox":true, "production":false}));

    let token = backend::apns::digest(&Uuid::new_v4().to_string());
    let id = backend::apns::digest(&format!("sandbox:{token}"));
    let secret = "a".repeat(64);
    let other_secret = "b".repeat(64);
    let revoke = |id: &str, secret: &str| json!({"id":id, "revocation_secret":secret});
    let row = || ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.clone())).first();

    let mut unsupported = registration(&token, &secret);
    unsupported["environment"] = "production".into();
    assert_eq!(
        post(&app, REGISTER, Some(&alice), unsupported)
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    // 同じ端末・同じ解除情報でも、期限を更新して登録を継続できる。
    for age in [540, 0] {
        let mut request = registration(&token, &secret);
        request["created_at"] = (jiff::Timestamp::now().as_second() - age).into();
        let result = post(&app, REGISTER, Some(&alice), request).await?;
        assert_eq!(result.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&to_bytes(result.into_body(), 4096).await?)?;
        assert_eq!(body["id"], id);
    }
    let test_request = |id: &str, secret: &str| json!({"id":id, "revocation_secret":secret, "test_id":Uuid::new_v4().to_string()});
    assert_eq!(
        post(&secured, TEST, None, test_request(&id, &secret))
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for (user, device, capability) in [
        (&bob, id.clone(), &secret),
        (&alice, id.clone(), &other_secret),
        (&alice, "e".repeat(64), &secret),
    ] {
        assert_eq!(
            post(&app, TEST, Some(user), test_request(&device, capability))
                .await?
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    // Appleに通信せず、同じ端末でのテスト連打を拒否する経路を確認する。
    toasty::sql::statement("UPDATE apns_subscriptions SET last_test_at=now() WHERE id=$1")
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        post(&app, TEST, Some(&alice), test_request(&id, &secret))
            .await?
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    for (user, capability) in [(&bob, &secret), (&alice, &other_secret)] {
        assert_eq!(
            post(&app, REGISTER, Some(user), registration(&token, capability))
                .await?
                .status(),
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        post(&secured, REVOKE, None, revoke(&id, &other_secret))
            .await?
            .status(),
        StatusCode::OK
    );
    assert!(row().exec(&mut db).await?.is_some());
    for _ in 0..2 {
        assert_eq!(
            post(&secured, REVOKE, None, revoke(&id, &secret))
                .await?
                .status(),
            StatusCode::OK
        );
        assert!(row().exec(&mut db).await?.is_none());
    }
    // ログアウトの後に届いた登録でも、古い解除情報を復活させない。
    assert_eq!(
        post(&app, REGISTER, Some(&alice), registration(&token, &secret))
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    let delayed_token = backend::apns::digest(&Uuid::new_v4().to_string());
    let delayed_id = backend::apns::digest(&format!("sandbox:{delayed_token}"));
    assert_eq!(
        post(&secured, REVOKE, None, revoke(&delayed_id, &secret))
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post(
            &app,
            REGISTER,
            Some(&alice),
            registration(&delayed_token, &secret)
        )
        .await?
        .status(),
        StatusCode::CONFLICT
    );

    // 前のアカウントから遅れて届いた解除で、新しい登録を消さない。
    let fresh = "c".repeat(64);
    assert_eq!(
        post(&app, REGISTER, Some(&bob), registration(&token, &fresh))
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post(&secured, REVOKE, None, revoke(&id, &secret))
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(row().exec(&mut db).await?.unwrap().user_id, bob);
    // 解除用の記録が満杯でも、正しい所有者の解除は配信を停止して再登録も防ぐ。
    let busy_token = backend::apns::digest(&Uuid::new_v4().to_string());
    let busy_id = backend::apns::digest(&format!("sandbox:{busy_token}"));
    assert_eq!(
        post(
            &app,
            REGISTER,
            Some(&bob),
            registration(&busy_token, &fresh)
        )
        .await?
        .status(),
        StatusCode::OK
    );
    let prefix = format!("apns-capacity-{bob}-");
    toasty::sql::statement("INSERT INTO apns_revocations(id,revoked_at) SELECT $1 || n::text,now() FROM generate_series(1,10000) n")
        .bind(prefix.as_str()).exec(&mut db).await?;
    let unknown = backend::apns::digest(&Uuid::new_v4().to_string());
    let saturated = post(&secured, REVOKE, None, revoke(&unknown, &fresh))
        .await?
        .status();
    let known = post(&secured, REVOKE, None, revoke(&busy_id, &fresh))
        .await?
        .status();
    toasty::sql::statement("DELETE FROM apns_revocations WHERE starts_with(id,$1)")
        .bind(prefix.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(saturated, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(known, StatusCode::OK);
    assert_eq!(
        post(
            &app,
            REGISTER,
            Some(&bob),
            registration(&busy_token, &fresh)
        )
        .await?
        .status(),
        StatusCode::CONFLICT
    );
    // 未登録端末からの解除要求が上限に達しても、正規端末は直ちに配信を停止できる。
    let mut limited = false;
    for _ in 0..=120 {
        let unknown = backend::apns::digest(&Uuid::new_v4().to_string());
        let response = post(&secured, REVOKE, None, revoke(&unknown, &fresh)).await?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert!(limited);
    for _ in 0..2 {
        assert_eq!(
            post(&secured, REVOKE, None, revoke(&id, &fresh))
                .await?
                .status(),
            StatusCode::OK
        );
    }
    assert!(row().exec(&mut db).await?.is_none());
    assert_eq!(
        post(&app, REGISTER, Some(&bob), registration(&token, &fresh))
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    Ok(())
}
