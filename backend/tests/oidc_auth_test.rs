use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use backend::{
    config::{Config, OidcConfig},
    db::init_db,
    models::User,
    routes::create_router,
    state::AppState,
};

const TEST_KEY1_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDLc9Dz1mwphgTR
BIphsGZ6Pqhy4Lu01ByPPcWybrOgEK/wyienr+xCR0KMFU9xyPCGiY0/BSC5rSJb
/gy2I3zBqYq/Iki12P2J6chQwRbTANgHqs7NU1fKQWIo0QtqUkbiSbPRfHHOzRz7
aKStQk8WLJhDEZv8BhG1jH3ZrPYPHRDftmIGalAItjqNGNIPCOgVDrSs9hIJtjBZ
0dTPHMtz9EQebBaQxv7mjIrAK0+k9uRD/DIC2kpDPW9gNiP3IuaCgcPbmA6hC0PB
kHnfprUhHqEWx8s1xT8GSjVufuvOP8ubqRGPoq42Wy4K9kZAfPfSklj6o7yIMtS/
WPH2ITxDAgMBAAECggEAJjMTq4S6AD3ij6Arn1Xwl8lF9XPOqpAc4KQ3h14Tqd0Y
O7ypH2n/kR4yF3000mdpCcmv1IJQ96vTRLR5jIUzOmVr4KE+R657RDa53reyYmHJ
Lf9bHuAkHpySIpnFa+wQ1RIDijS0ZaDLbUTjjohhswcie41F3VcCYjUDuCgqdwGn
D6XMNU+89NMzSkENPM83SgmVTm18vmTmH+L2GwVt6mIHPectFdbM9NkjxHlNaXRE
iIPKUu0V2l20XAzoCIabwpkzs7e46Rt14s5iBYQH1y0LsE1knQylpcsqE0n0AIdR
nIX1tVXJr7MS8VlRx8RCtbCete3MY9cJk3Er8EWruQKBgQD5AubiVdOKJ5uHkJhp
D5LnescTgHJN0Ok1dTQo6QUd0BfdU4NeDKPICglQMyNYvZdz8tUv+PrTg4Cf52eS
m/5oz0Nf7mzshIki+nb4j6OHsn93MkakwaYKJgK8no+RX4HwbZ5PHwymziFtGLSO
HR9E8iSUlNAwmovoHjR7UQ10lQKBgQDRKZUP0ksfPnN4RDUxBAdMngGoH1Q0Jx/Q
H5L/uH+j0wFWYRai+67RLRDacSF5JTXpwff+N2fSTOfXB2/3vz2d8WM/761Qp5mN
2bbP2hIA8puxLWbliMEtIKxVyev/ZxQxMGw0BsjaoCc5aqsCKLDthpZ+gPyLtP4L
jkx7xjofdwKBgA5QG2NRGZqsUiVFq20GhyQlC4JUdL2CT5VSClB7rF6dUnHXVgq2
4UcjRIvGlLaKF0WD5LsgLZKw0VjfnTVxH/t4RmAiPUKPD67iBKqe3wBywx7f1e2s
b9XPf4LF/myVt8tREbNsjnacDJHPxLkSdXSqEm4z64+PopESA2Q/MW3NAoGBAKTJ
nv/dDYRrHmwlSMb6Rontfp6nOB7yqBS/m3/PKvi1+DrGuR/lLGDuoXWIZffdAjHN
uQVXP3unTYzS9jr5N6FlGW249hIyST9BxM7THxYSDem0PHogmRllwYDBWiZZXsY2
QTloEvxz6xNIX7yOtH4eaBj4pH+pM/GExetKTGaXAoGAbTbl6agiq9PAFIgmYHaH
CaJducfFAlWeSQsx9N6qVEY/6sPFv6ZoevF86n0hlBTyb7qap87kHyfBJZR3P1By
lEl8A6VuBIAnpqkbJz9nT7xsS/1UbKuFQ5jMH9qz20KZmD5YTZ/6zre3zLTdJK5A
WHYmwlj7VcIctm9ecC5nEQY=
-----END PRIVATE KEY-----
"#;

const TEST_KEY2_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDFFjm/LOFGEWM4
7Uw/SM3jo2gX3mXY/iQ4b0Cbo7V3VWRBC3He94m+a8C5DWIFR/R/4EuK5tChSlO8
y42YS0B6uYJhBb59q2qQR6CvbsKEPTcfajZmbOj9FpfO7oAxkW1Q10eb5/AaKF3v
AGB3SrCwXvWFofM0QIvD8LUQm1goVQCXQQx5C7A8SLRJwkkcN4mgMK5Kw/ngpwsP
aRC38wkYv9FFeNnhzIhJWU/KsXZ1C3Vvflp7VD0h7KZYr2L3OM/xTZc3Yx3u1DPq
Us1+R6MRJOTbEUFwiC7V/z/Z7wmLs53e9tYnV9AcZZczxenSoPaNe+t3E9eS5EkJ
WfbhNEILAgMBAAECggEAExlfST6BKjgfJHIilHNquGeqe72qowel3O4zFaIcwM6+
o2sHgvL8Hz7z8zrvx9E8SXQj/fb2OtsoGEClnR+VxQ7bKqnS1vzYm9jFIpsaMJR6
FgpF8VEWlwgsd/o25BbSwCXMo7grVW2aBMJFlunfC4h4RJX4KpUitj9KXKyhM281
syc5KBizW8PYKo/qkyw3ymN82ChCIILIi2Ck3kp9ZW1QdIKIHFWjCGDa6chdmjir
Vgbw3dqoK4Y4ArXB7d5g9PxjOB3xAHkK7QJ+p41nS8y2hB2q6HJfvJ0CgIJN+zcy
3fyZSm5tewbFXw86DSAnVBToG2Of8HrP34LWWb0usQKBgQDxzCWD7FzBP3NDnVGq
f4Er0KzpK7iyjNdpk49xDWQLuV+YdedB/u/WFUA9zXtLTV5+hI1XlTtBIZtSrvMW
0wbhzd8oSIygWVPgsrb+A3v7GU6jD57xICX0pYgykfwcTpqcNS6II8Ih1rnzC1Sh
mvReOkbZG3+sLn4mkG/rtvh00wKBgQDQqcaCkivKkfeqN8iRRDP4NV0GJ0NLz5o4
O0SiFl6hy3R2KuiYNst4MuSoRjMCTHdphwFXMUCGsiTDpKp+3VxC5Ik69G8sv585
x8aNFuiQQiponmoW/ZQU54REuYvgzXnTDxl5oG/Voqb9XiZYC2g2m4LwunngAsgS
A20h/yma6QKBgARBBDPy95rKw/tfjC1EuwALVs9vXnt9sNiBqRdjFeyC4WSsRlpJ
4uxQSWYM9GtZvBzbtiThUj6OGc51l39CTS+x0IhD5n13tNsSKdgENa0wP8rsnm49
k41fUCsbmGTjGSNAaEYq4uJ3haP6Ag4pzLzPp6lwK2nbE4sYJW/vMyXTAoGALwkm
M0ap0mbm8hrS/lPmdxvoVTvCgrNQaz6I8RSjzkrjfMOdXLp1USn/uyLf2rPL8yC4
n6P9jJ9ZhDR5LlHNu1Jy0LRK30ZuqQKsch9o2yPfFWgoN/h/SdsKSMeejVUkEfyE
XJNawkiA5fo0IIKpwi157bPLR0C5Qr48+n3fLgkCgYBpeQGyUgQvLuY0MtUeXxkX
lqbQpmZfApaeVITHbJxuf6FtgaEjlLCoqPqzsu7hyuaZsIPsSmZfB4iJX89W9yGR
0/wdoE9mPTme1cFZjx4N/+/Va+7GfdsRmcOSqyVHIqggTJS3Bx0TxHP1kUWSbT+c
PEvmSOv5UXUKKMXCu7pfDQ==
-----END PRIVATE KEY-----
"#;

fn key1_jwk() -> Value {
    json!({
        "kty": "RSA",
        "n": "y3PQ89ZsKYYE0QSKYbBmej6ocuC7tNQcjz3Fsm6zoBCv8Monp6_sQkdCjBVPccjwhomNPwUgua0iW_4MtiN8wamKvyJItdj9ienIUMEW0wDYB6rOzVNXykFiKNELalJG4kmz0Xxxzs0c-2ikrUJPFiyYQxGb_AYRtYx92az2Dx0Q37ZiBmpQCLY6jRjSDwjoFQ60rPYSCbYwWdHUzxzLc_REHmwWkMb-5oyKwCtPpPbkQ_wyAtpKQz1vYDYj9yLmgoHD25gOoQtDwZB536a1IR6hFsfLNcU_Bko1bn7rzj_Lm6kRj6KuNlsuCvZGQHz30pJY-qO8iDLUv1jx9iE8Qw",
        "e": "AQAB",
        "kid": "test-key-1",
        "use": "sig",
        "alg": "RS256"
    })
}

fn key2_jwk() -> Value {
    json!({
        "kty": "RSA",
        "n": "xRY5vyzhRhFjOO1MP0jN46NoF95l2P4kOG9Am6O1d1VkQQtx3veJvmvAuQ1iBUf0f-BLiubQoUpTvMuNmEtAermCYQW-fatqkEegr27ChD03H2o2Zmzo_RaXzu6AMZFtUNdHm-fwGihd7wBgd0qwsF71haHzNECLw_C1EJtYKFUAl0EMeQuwPEi0ScJJHDeJoDCuSsP54KcLD2kQt_MJGL_RRXjZ4cyISVlPyrF2dQt1b35ae1Q9IeymWK9i9zjP8U2XN2Md7tQz6lLNfkejESTk2xFBcIgu1f8_2e8Ji7Od3vbWJ1fQHGWXM8Xp0qD2jXvrdxPXkuRJCVn24TRCCw",
        "e": "AQAB",
        "kid": "test-key-2",
        "use": "sig",
        "alg": "RS256"
    })
}

#[allow(clippy::too_many_arguments)]
fn generate_token(
    pem: &str,
    kid: &str,
    iss: &str,
    aud: &str,
    sub: &str,
    preferred_username: Option<&str>,
    name: Option<&str>,
    scope: Option<&str>,
    exp_offset_sec: i64,
) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());

    let now = jiff::Timestamp::now().as_second();
    let exp = (now + exp_offset_sec) as u64;

    let claims = json!({
        "iss": iss,
        "aud": aud,
        "sub": sub,
        "exp": exp,
        "preferred_username": preferred_username,
        "name": name,
        "scope": scope,
    });

    let enc_key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &enc_key).unwrap()
}

async fn setup_test_app(mock_server: &MockServer) -> (axum::Router, AppState, OidcConfig) {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let issuer = mock_server.uri();
    let audience = "zoo-client-test".to_string();
    let jwks_url = format!("{}/.well-known/jwks.json", mock_server.uri());

    let oidc_config = OidcConfig {
        issuer: issuer.clone(),
        audience: audience.clone(),
        jwks_url,
        scope_student: "student".to_string(),
        scope_freshman: "freshman".to_string(),
        scope_alumni: "alumni".to_string(),
    };

    let mut config = Config::from_env();
    config.oidc = Some(oidc_config.clone());

    let db = init_db(&config.database_url).await.unwrap();
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    (app, state, oidc_config)
}

#[tokio::test]
async fn test_oidc_jit_provisioning_and_user_profile_flow() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    // JWKS モック (key1 のみ)
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server).await;

    let sub_1 = format!("kanidm-sub-{}", uuid::Uuid::new_v4().simple());
    let username = format!("alice_{}", &uuid::Uuid::new_v4().simple().to_string()[..6]);
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &sub_1,
        Some(&username),
        Some("Alice In Wonderland"),
        Some("openid profile student"),
        3600,
    );

    // 1. 初回ログイン (JIT プロビジョニング)
    let req = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    let user_id = user_json["id"].as_str().expect("id must be string");
    assert_eq!(user_json["user_name"], username);
    assert_eq!(user_json["display_name"], "Alice In Wonderland");
    assert_eq!(user_json["affiliation"], "student");

    // 2. 再ログインで同じ内部ユーザー ID に解決されること
    let req2 = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp2 = app.clone().oneshot(req2).await?;
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = to_bytes(resp2.into_body(), usize::MAX).await?;
    let user_json2: Value = serde_json::from_slice(&body2)?;
    assert_eq!(user_json2["id"].as_str(), Some(user_id));

    // 3. 区分変更の反映 (student -> alumni)
    let updated_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &sub_1,
        Some(&username),
        Some("Alice In Wonderland"),
        Some("openid profile alumni"),
        3600,
    );

    let req3 = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {updated_token}"))
        .body(Body::empty())?;

    let resp3 = app.clone().oneshot(req3).await?;
    assert_eq!(resp3.status(), StatusCode::OK);
    let body3 = to_bytes(resp3.into_body(), usize::MAX).await?;
    let user_json3: Value = serde_json::from_slice(&body3)?;
    assert_eq!(user_json3["id"].as_str(), Some(user_id));
    assert_eq!(user_json3["affiliation"], "alumni");

    // 4. PATCH /api/users/me (プロフィールの本人限定更新と txid 返却)
    let patch_payload = json!({
        "display_name": "Alice The Graduate",
    });
    let req_patch = Request::builder()
        .method("PATCH")
        .uri("/api/users/me")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {updated_token}"))
        .body(Body::from(serde_json::to_vec(&patch_payload)?))?;

    let resp_patch = app.clone().oneshot(req_patch).await?;
    assert_eq!(resp_patch.status(), StatusCode::OK);
    let patch_body = to_bytes(resp_patch.into_body(), usize::MAX).await?;
    let patch_json: Value = serde_json::from_slice(&patch_body)?;
    assert_eq!(patch_json["data"]["display_name"], "Alice The Graduate");
    let txid = patch_json["txid"].as_str().expect("txid required");
    assert!(!txid.is_empty());

    // 5. GET /api/users/me で更新が反映されていること
    let req_get = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {updated_token}"))
        .body(Body::empty())?;
    let resp_get = app.clone().oneshot(req_get).await?;
    let body_get = to_bytes(resp_get.into_body(), usize::MAX).await?;
    let json_get: Value = serde_json::from_slice(&body_get)?;
    assert_eq!(json_get["display_name"], "Alice The Graduate");

    Ok(())
}

#[tokio::test]
async fn test_oidc_token_validation_failures() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server).await;

    // 1. 署名不正 (key2 で署名したトークンを、key1 しか登録されていない JWKS に対して送信)
    let invalid_sig_token = generate_token(
        TEST_KEY2_PEM,
        "test-key-1", // kid は一致させるが鍵が違う
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-invalid-sig",
        Some("bob"),
        Some("Bob"),
        None,
        3600,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {invalid_sig_token}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2. 期限切れ (exp < now)
    let expired_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-expired",
        Some("bob"),
        Some("Bob"),
        None,
        -100, // 100秒前
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {expired_token}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Issuer 不一致
    let wrong_iss_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        "https://evil-idp.example.com",
        &oidc_cfg.audience,
        "sub-wrong-iss",
        Some("bob"),
        Some("Bob"),
        None,
        3600,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {wrong_iss_token}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 4. Audience 不一致
    let wrong_aud_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        "wrong-audience",
        "sub-wrong-aud",
        Some("bob"),
        Some("Bob"),
        None,
        3600,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {wrong_aud_token}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 5. トークン欠落
    let req = Request::builder()
        .uri("/api/users/me")
        .body(Body::empty())?;
    // Dev Auth が有効なテスト環境ではデフォルト開発ユーザーになるが、
    // ENVIRONMENT=production のときは 401 になる (テスト 4 で確認)
    let _ = app.clone().oneshot(req).await?;

    Ok(())
}

#[tokio::test]
async fn test_oidc_key_rotation() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    // 最初は key1 と key2 の両方をエンドポイントで返却できるようにする
    let jwks_body = json!({ "keys": [key1_jwk(), key2_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server).await;

    // key2 で署名された新しいトークン
    let carol_username = format!("carol_{}", &uuid::Uuid::new_v4().simple().to_string()[..6]);
    let key2_token = generate_token(
        TEST_KEY2_PEM,
        "test-key-2",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("sub-rotation-{}", uuid::Uuid::new_v4().simple()),
        Some(&carol_username),
        Some("Carol"),
        Some("openid profile freshman"),
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {key2_token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    assert_eq!(user_json["user_name"], carol_username);
    assert_eq!(user_json["affiliation"], "freshman");

    Ok(())
}

#[tokio::test]
async fn test_production_environment_fail_closed_guards() -> anyhow::Result<()> {
    unsafe {
        std::env::set_var("ENVIRONMENT", "production");
        std::env::remove_var("DEV_AUTH_ENABLED");
    }

    let mock_server = MockServer::start().await;
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server).await;

    let test_uuid = "44444444-4444-4444-4444-444444444444";

    // 1. 本番で X-Dev-User は拒否
    let req1 = Request::builder()
        .uri("/api/users/me")
        .header("X-Dev-User", test_uuid)
        .body(Body::empty())?;
    let resp1 = app.clone().oneshot(req1).await?;
    assert_eq!(resp1.status(), StatusCode::UNAUTHORIZED);

    // 2. 本番でダミー Bearer <UUID> は拒否
    let req2 = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {test_uuid}"))
        .body(Body::empty())?;
    let resp2 = app.clone().oneshot(req2).await?;
    assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);

    // 3. 本番でヘッダなしは拒否
    let req3 = Request::builder()
        .uri("/api/users/me")
        .body(Body::empty())?;
    let resp3 = app.clone().oneshot(req3).await?;
    assert_eq!(resp3.status(), StatusCode::UNAUTHORIZED);

    // 4. 本番で POST /api/users (直接作成) は拒否
    let payload = json!({
        "user_name": "direct_user",
        "display_name": "Direct User",
    });
    let req4 = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload)?))?;
    let resp4 = app.clone().oneshot(req4).await?;
    assert_eq!(resp4.status(), StatusCode::FORBIDDEN);

    // 5. 正規の OIDC Bearer Token なら本番でもアクセス成功
    let valid_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("sub-prod-{}", uuid::Uuid::new_v4().simple()),
        Some("prod_user"),
        Some("Prod User"),
        Some("openid profile student"),
        3600,
    );
    let req5 = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {valid_token}"))
        .body(Body::empty())?;
    let resp5 = app.clone().oneshot(req5).await?;
    assert_eq!(resp5.status(), StatusCode::OK);

    // クリーンアップ
    unsafe {
        std::env::remove_var("ENVIRONMENT");
    }

    Ok(())
}

#[tokio::test]
async fn test_oidc_jit_concurrent_requests_no_conflict() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, state, oidc_cfg) = setup_test_app(&mock_server).await;

    let shared_sub = format!("concurrent-sub-{}", uuid::Uuid::new_v4().simple());
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &shared_sub,
        Some("concurrent_user"),
        Some("Concurrent User"),
        Some("openid profile student"),
        3600,
    );

    // 同一 sub のトークンで 5 並行リクエストを同時に送信
    let mut handles = Vec::new();
    for _ in 0..5 {
        let app_clone = app.clone();
        let tok = token.clone();
        handles.push(tokio::spawn(async move {
            let req = Request::builder()
                .uri("/api/users/me")
                .header("Authorization", format!("Bearer {tok}"))
                .body(Body::empty())
                .unwrap();
            app_clone.oneshot(req).await.unwrap()
        }));
    }

    for h in handles {
        let resp = h.await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // users テーブル内に shared_sub を持つユーザーがちょうど 1 人だけ存在することを確認
    let mut db = state.db.clone();
    let users = User::filter(
        User::fields()
            .oidc_iss()
            .eq(Some(oidc_cfg.issuer.clone()))
            .and(User::fields().oidc_sub().eq(Some(shared_sub.clone()))),
    )
    .exec(&mut db)
    .await?;

    assert_eq!(users.len(), 1, "Exactly one user should be provisioned");

    Ok(())
}
