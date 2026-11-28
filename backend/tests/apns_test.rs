#[path = "support/thread_db.rs"]
mod test_db;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use backend::{db::init_db, models::ApnsSubscription, routes::create_router, state::AppState};
use futures_util::FutureExt;
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
    // 共有の開発DBに残った解除記録が、容量とプロセス側レートの検証に混ざらないようにする。
    let fixture = test_db::ThreadDb::new().await?;
    let result =
        std::panic::AssertUnwindSafe(check_registration_with_config(fixture.config.clone()))
            .catch_unwind()
            .await;
    let cleanup = fixture.cleanup().await;
    result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
    cleanup
}

async fn check_registration_with_config(mut config: backend::config::Config) -> anyhow::Result<()> {
    let mut url = reqwest::Url::parse(&config.database_url)?;
    let parameters: Vec<_> = url
        .query_pairs()
        .map(|(key, value)| {
            let value = if key == "options" {
                format!("{value} -cTimeZone=Asia/Tokyo")
            } else {
                value.into_owned()
            };
            (key.into_owned(), value)
        })
        .collect();
    url.query_pairs_mut().clear().extend_pairs(parameters);
    config.database_url = url.to_string();
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
    assert_eq!(
        body,
        json!({"sandbox":true, "production":false, "voip":true})
    );

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
    for issued in [i64::MIN, -62_198_755_200, i64::MAX] {
        let mut out_of_range = registration(&token, &secret);
        out_of_range["created_at"] = issued.into();
        assert_eq!(
            post(&app, REGISTER, Some(&alice), out_of_range)
                .await?
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    // 解除記録の読み取りが待たされる間に、解除記録と元の登録要求が共に期限切れになる。
    let mut blocker = db.clone();
    let mut tx = blocker.transaction().await?;
    toasty::sql::statement("LOCK TABLE apns_revocations IN ACCESS EXCLUSIVE MODE")
        .exec(&mut tx)
        .await?;
    let marker = backend::apns::digest(&format!("{id}:{}", backend::apns::digest(&secret)));
    toasty::sql::statement(
        "INSERT INTO apns_revocations(id,revoked_at) VALUES (
            $1,(clock_timestamp() AT TIME ZONE 'UTC') - interval '11 minutes' + interval '0.5 seconds')",
    ).bind(marker.as_str()).exec(&mut tx).await?;
    let mut last_second = registration(&token, &secret);
    last_second["created_at"] = (jiff::Timestamp::now().as_second() - 600).into();
    let mut delayed = std::pin::pin!(post(&app, REGISTER, Some(&alice), last_second));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1200), &mut delayed)
            .await
            .is_err()
    );
    tx.commit().await?;
    assert_eq!(delayed.await?.status(), StatusCode::BAD_REQUEST);
    assert!(row().exec(&mut db).await?.is_none());
    // 確認が古い要求は同じ応答を返しても、端末を再確認した時刻を進めない。
    let now = jiff::Timestamp::now().as_second();
    let register_at = |issued: i64| {
        let mut request = registration(&token, &secret);
        request["created_at"] = issued.into();
        request
    };
    let utc = |s| {
        jiff::Timestamp::from_second(s)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .datetime()
    };
    for (issued, expected) in [
        (now - 540, now - 540),
        (now - 540, now - 540),
        (now - 400, now - 400),
        (now - 540, now - 400),
    ] {
        let result = post(&app, REGISTER, Some(&alice), register_at(issued)).await?;
        assert_eq!(result.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&to_bytes(result.into_body(), 4096).await?)?;
        assert_eq!(body["id"], id);
        assert_eq!(
            row().exec(&mut db).await?.unwrap().created_at,
            utc(expected)
        );
    }
    // 410の処理が最初に完了した場合も、端末の再確認が失効より新しくなるまで登録し直せない。
    let invalid_at = now - 300;
    toasty::sql::statement("DELETE FROM apns_subscriptions WHERE id=$1")
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("INSERT INTO apns_revocations(id,revoked_at,invalidated_at) VALUES($1, statement_timestamp() AT TIME ZONE 'UTC', $2::text::timestamp)")
        .bind(id.as_str()).bind(utc(invalid_at).to_string()).exec(&mut db).await?;
    for issued in [now - 540, invalid_at] {
        assert_eq!(
            post(&app, REGISTER, Some(&alice), register_at(issued))
                .await?
                .status(),
            StatusCode::CONFLICT
        );
        assert!(row().exec(&mut db).await?.is_none());
    }
    assert_eq!(
        post(&app, REGISTER, Some(&alice), register_at(now))
            .await?
            .status(),
        StatusCode::OK
    );
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
    // 端末の失効、誤った解除情報、正規の解除を1件ずつ数え、解除の再送では増えない。
    assert_eq!(
        toasty::sql::query("SELECT 1::text FROM apns_revocation_capacity WHERE id AND records=3")
            .exec(&mut db)
            .await?
            .len(),
        1
    );
    let revoke_id = backend::apns::digest(&format!("{id}:{}", backend::apns::digest(&secret)));
    let utc_record = toasty::sql::query("SELECT 1::text FROM apns_revocations WHERE id=$1 AND abs(extract(epoch FROM revoked_at - (statement_timestamp() AT TIME ZONE 'UTC'))) < 30")
        .bind(revoke_id).exec(&mut db).await?;
    assert_eq!(utc_record.len(), 1);
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
    // 同じ世代がすでにAppleの410で削除されていても、ログアウトは正規の解除として扱う。
    let retired_token = backend::apns::digest(&Uuid::new_v4().to_string());
    let retired_id = backend::apns::digest(&format!("sandbox:{retired_token}"));
    let retired_capability =
        backend::apns::digest(&format!("{retired_id}:{}", backend::apns::digest(&fresh)));
    toasty::sql::statement(
        "INSERT INTO apns_revocations(id,revoked_at,invalidated_at)
         SELECT id, statement_timestamp() AT TIME ZONE 'UTC', $3::text::timestamp
         FROM (VALUES ($1::text), ($2::text)) AS markers(id)",
    )
    .bind(retired_id.as_str())
    .bind(retired_capability)
    .bind(utc(invalid_at).to_string())
    .exec(&mut db)
    .await?;
    let prefix = format!("apns-capacity-{bob}-");
    toasty::sql::statement("INSERT INTO apns_revocations(id,revoked_at) SELECT $1 || n::text,statement_timestamp() AT TIME ZONE 'UTC' FROM generate_series(1,10000) n")
        .bind(prefix.as_str()).exec(&mut db).await?;
    let unknown = backend::apns::digest(&Uuid::new_v4().to_string());
    let saturated = post(&secured, REVOKE, None, revoke(&unknown, &fresh))
        .await?
        .status();
    let known = post(&secured, REVOKE, None, revoke(&busy_id, &fresh))
        .await?
        .status();
    assert_eq!(
        post(&secured, REVOKE, None, revoke(&retired_id, &fresh))
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post(
            &app,
            REGISTER,
            Some(&bob),
            registration(&retired_token, &fresh)
        )
        .await?
        .status(),
        StatusCode::CONFLICT
    );
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
    // 期限切れが1回の回収件数を超えていても、その行は有効な解除記録の上限に含めない。
    toasty::sql::statement(
        "INSERT INTO apns_revocations(id,revoked_at)
         SELECT $1 || n::text,(statement_timestamp() AT TIME ZONE 'UTC') - interval '12 minutes'
         FROM generate_series(1,10200) n",
    )
    .bind(prefix.as_str())
    .exec(&mut db)
    .await?;
    assert_eq!(
        post(&secured, REVOKE, None, revoke(&unknown, &fresh))
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        toasty::sql::query(
            "SELECT 1::text FROM apns_revocations WHERE starts_with(id,$1) HAVING count(*)=10072"
        )
        .bind(prefix.as_str())
        .exec(&mut db)
        .await?
        .len(),
        1
    );
    toasty::sql::statement("DELETE FROM apns_revocations WHERE starts_with(id,$1)")
        .bind(prefix.as_str())
        .exec(&mut db)
        .await?;
    // 未登録端末からの解除要求が上限に達しても、正規端末は直ちに配信を停止できる。
    // 間違った解除情報、登録前の解除、DB上限2回の確認で4枠を消費済み。
    for _ in 4..120 {
        let unknown = backend::apns::digest(&Uuid::new_v4().to_string());
        let response = post(&secured, REVOKE, None, revoke(&unknown, &fresh)).await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let over_limit = backend::apns::digest(&Uuid::new_v4().to_string());
    assert_eq!(
        post(&secured, REVOKE, None, revoke(&over_limit, &fresh))
            .await?
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
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
    // VoIPも既存の所有者確認・解除能力を使い、通常通知のテスト送信には混ぜない。
    let caller = Uuid::new_v4().to_string();
    assert_eq!(
        post(
            &app,
            "/api/users",
            None,
            json!({"id":caller,"user_name":format!("voip_{caller}"),"display_name":"通話テスト"})
        )
        .await?
        .status(),
        StatusCode::OK
    );
    let voip_token = backend::apns::digest(&Uuid::new_v4().to_string());
    let voip_secret = backend::apns::digest(&Uuid::new_v4().to_string());
    let mut body = registration(&voip_token, &voip_secret);
    body["kind"] = json!("voip");
    let response = post(&app, REGISTER, Some(&caller), body.clone()).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    let voip_id = backend::apns::PushKind::Voip.registration_id("sandbox", &voip_token);
    assert_eq!(result["id"], voip_id);
    assert_eq!(
        post(&app, REGISTER, Some(&bob), body.clone())
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(post(&app, TEST, Some(&caller),
        json!({"id":voip_id,"revocation_secret":voip_secret,"test_id":Uuid::new_v4().to_string()}))
        .await?.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        post(
            &secured,
            REVOKE,
            None,
            json!({"id":voip_id,"revocation_secret":voip_secret})
        )
        .await?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        post(&app, REGISTER, Some(&caller), body).await?.status(),
        StatusCode::CONFLICT
    );
    Ok(())
}
