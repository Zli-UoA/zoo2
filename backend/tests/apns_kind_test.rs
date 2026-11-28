#[path = "support/thread_db.rs"]
mod test_db;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{db::init_db, routes::create_router, state::AppState};
use futures_util::FutureExt;
use serde_json::{Value, json};
use tower::ServiceExt;

#[test]
fn registration_separates_push_kinds() -> anyhow::Result<()> {
    run_isolated("registration_separates_push_kinds", false)
}

#[path = "support/apns_kind_budget.rs"]
mod budget;

#[test]
#[ignore = "30万件・100万件のDBと、ZOO_APNS_KIND_BUDGET_OUTPUT・ZOO_APNS_KIND_BUDGET_MACHINEが必要"]
fn registration_kind_write_budget() -> anyhow::Result<()> {
    run_isolated("registration_kind_write_budget", true)
}

fn run_isolated(test_name: &str, performance: bool) -> anyhow::Result<()> {
    const CHILD: &str = "ZOO_APNS_KIND_TEST";
    if std::env::var_os(CHILD).is_none() {
        use openssl::{
            ec::{EcGroup, EcKey},
            nid::Nid,
            pkey::PKey,
        };
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
        let key = PKey::from_ec_key(EcKey::generate(&group)?)?;
        let status = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", test_name, "--include-ignored", "--nocapture"])
            .env(CHILD, "1")
            .env("APPLE_TEAM_ID", "ABCDEFGHIJ")
            .env("APNS_SANDBOX_KEY_ID", "KLMNOPQRST")
            .env(
                "APNS_SANDBOX_PRIVATE_KEY",
                String::from_utf8(key.private_key_to_pem_pkcs8()?)?,
            )
            .env("APNS_PRODUCTION_KEY_ID", "KLMNOPQRST")
            .env(
                "APNS_PRODUCTION_PRIVATE_KEY",
                String::from_utf8(key.private_key_to_pem_pkcs8()?)?,
            )
            .status()?;
        anyhow::ensure!(status.success(), "通知用途別の登録テストに失敗しました");
        return Ok(());
    }
    tokio::runtime::Runtime::new()?.block_on(async {
        let fixture = test_db::ThreadDb::new().await?;
        let result = std::panic::AssertUnwindSafe(async {
            if performance {
                budget::run(fixture.config.clone()).await
            } else {
                check_registration(fixture.config.clone()).await
            }
        })
        .catch_unwind()
        .await;
        let cleanup = fixture.cleanup().await;
        result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        cleanup
    })
}

async fn post(
    app: &Router,
    path: &str,
    user: Option<&str>,
    value: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(user) = user {
        request = request.header("X-Dev-User", user);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(value.to_string()))?)
        .await?;
    let status = response.status();
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    Ok((status, value))
}

const REGISTER: &str = "/api/push/apns/subscriptions";
const REVOKE: &str = "/api/push/apns/revoke";

fn registration(kind: &str, environment: &str, token: &str, secret: &str) -> Value {
    json!({"kind":kind,"device_token":token,"environment":environment,
        "revocation_secret":secret,"created_at":jiff::Timestamp::now().as_second()})
}

async fn check_registration(config: backend::config::Config) -> anyhow::Result<()> {
    use backend::apns::{PushKind, digest};
    let (client, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new_for_worker(db, config));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for user in [&alice, &bob] {
        let (status, _) = post(
            &app,
            "/api/users",
            None,
            json!({"id":user,"user_name":format!("kind_{user}"),"display_name":"通知用途の検証"}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }
    // 旧スキーマに戻し、既存行を保持した移行と、移行前の409を同じ経路で確認する。
    client.batch_execute("ALTER TABLE apns_subscriptions ADD CONSTRAINT apns_subscriptions_device_token_environment_key UNIQUE(device_token, environment);
        DROP INDEX apns_subscriptions_token_environment_kind_key").await?;
    let token = digest("移行前の通知トークン");
    let alert_secret = digest("通常通知の解除情報");
    let voip_secret = digest("通話着信の解除情報");
    let alert = registration("alert", "sandbox", &token, &alert_secret);
    let voip = registration("voip", "sandbox", &token, &voip_secret);
    let (status, _) = post(&app, REGISTER, Some(&alice), alert.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, error) = post(&app, REGISTER, Some(&alice), voip.clone()).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        error["error"],
        "Resource already exists or unique constraint was violated"
    );
    let snapshot = "SELECT row_to_json(s)::text FROM apns_subscriptions s ORDER BY id";
    let before: Vec<String> = client
        .query(snapshot, &[])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let (create, drop) = budget::migration_statements();
    for statement in [create, drop] {
        client.batch_execute(statement).await?;
    }
    let after: Vec<String> = client
        .query(snapshot, &[])
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        post(&app, REGISTER, Some(&alice), voip.clone()).await?.0,
        StatusCode::OK
    );
    let alert_id = PushKind::Alert.registration_id("sandbox", &token);
    let voip_id = PushKind::Voip.registration_id("sandbox", &token);
    for (body, id) in [(&alert, &alert_id), (&voip, &voip_id)] {
        for _ in 0..2 {
            let (status, result) = post(&app, REGISTER, Some(&alice), body.clone()).await?;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(result["id"], *id);
        }
        assert_eq!(
            post(&app, REGISTER, Some(&bob), body.clone()).await?.0,
            StatusCode::CONFLICT
        );
        let mut wrong = body.clone();
        wrong["revocation_secret"] = digest("別の解除情報").into();
        assert_eq!(
            post(&app, REGISTER, Some(&alice), wrong).await?.0,
            StatusCode::CONFLICT
        );
    }
    // 一方の解除能力で他方を解除したり、解除済みの登録を再送で復活させたりできない。
    assert_eq!(
        post(
            &app,
            REVOKE,
            None,
            json!({"id":voip_id,"revocation_secret":alert_secret})
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM apns_subscriptions", &[])
            .await?
            .get::<_, i64>(0),
        2
    );
    assert_eq!(
        post(
            &app,
            REVOKE,
            None,
            json!({"id":alert_id,"revocation_secret":alert_secret})
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        post(&app, REGISTER, Some(&alice), alert.clone()).await?.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        post(&app, REGISTER, Some(&alice), voip.clone()).await?.0,
        StatusCode::OK
    );
    let row = client
        .query_one(
            "SELECT user_id,revocation_hash FROM apns_subscriptions WHERE id=$1",
            &[&voip_id],
        )
        .await?;
    assert_eq!(row.get::<_, String>(0), alice);
    assert_eq!(row.get::<_, String>(1), digest(&voip_secret));
    let mut fresh = alert;
    fresh["revocation_secret"] = digest("解除後の新しい登録").into();
    assert_eq!(
        post(&app, REGISTER, Some(&bob), fresh).await?.0,
        StatusCode::OK
    );

    // 登録順・環境・最大長のトークンに依存せず、同時の再試行でも2行だけになる。
    for environment in ["sandbox", "production"] {
        for first in ["alert", "voip"] {
            let token = digest(&format!("{environment}:{first}")).repeat(8);
            let second = if first == "alert" { "voip" } else { "alert" };
            for kind in [first, second] {
                let body = registration(kind, environment, &token, &digest(kind));
                assert_eq!(
                    post(&app, REGISTER, Some(&alice), body).await?.0,
                    StatusCode::OK
                );
            }
            let token = digest(&format!("同時登録:{environment}:{first}")).repeat(8);
            let responses = futures_util::future::try_join_all((0..8).map(|n| {
                let kind = if n % 2 == 0 { "alert" } else { "voip" };
                post(
                    &app,
                    REGISTER,
                    Some(&alice),
                    registration(kind, environment, &token, &digest(kind)),
                )
            }))
            .await?;
            assert!(
                responses
                    .iter()
                    .all(|(status, _)| *status == StatusCode::OK)
            );
            assert_eq!(client.query_one("SELECT count(*) FROM apns_subscriptions WHERE device_token=$1 AND environment=$2", &[&token,&environment]).await?.get::<_,i64>(0),2);
        }
    }
    // ID規約に従わない旧データも通常通知として扱い、同用途の重複制約を残す。
    let error = client
        .execute(
            "INSERT INTO apns_subscriptions(id,user_id,device_token,environment,revocation_hash,created_at)
        VALUES('非正規のID',$1,$2,'sandbox','検証用',now())",
            &[&bob, &token],
        )
        .await
        .unwrap_err();
    let error = error.as_db_error().unwrap();
    assert_eq!(
        error.code(),
        &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
    );
    assert_eq!(
        error.constraint(),
        Some("apns_subscriptions_token_environment_kind_key")
    );
    Ok(())
}
