use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

use backend::{
    config::{Config, OidcConfig},
    db::init_db,
    models::{User, UserIdentity},
    routes::create_router,
    state::AppState,
    validation::validate_user_name,
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

const TEST_ED25519_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIA0bID3qxIoHZTqzIn9bRmUiRLn6RyGupe/S6WE1CL7/
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

fn ed25519_jwk(include_alg: bool) -> Value {
    let mut jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": "cB-qRdJ9-hGN7VUdA71a5iLBDK9JKRIMVHe8_F3Dm3Y",
        "kid": "test-ed-key-1",
        "use": "sig"
    });
    if include_alg {
        jwk["alg"] = json!("EdDSA");
    }
    jwk
}

#[allow(clippy::too_many_arguments)]
fn encode_token(
    alg: Algorithm,
    encoding_key: &EncodingKey,
    kid: &str,
    typ: Option<&str>,
    iss: &str,
    aud: &str,
    sub: &str,
    preferred_username: Option<&str>,
    name: Option<&str>,
    scope: Option<&str>,
    exp_offset_sec: i64,
    nbf_offset_sec: Option<i64>,
) -> String {
    let mut header = Header::new(alg);
    header.kid = Some(kid.to_string());
    header.typ = typ.map(|t| t.to_string());

    let now = jiff::Timestamp::now().as_second();
    let exp = (now + exp_offset_sec) as u64;
    let nbf = nbf_offset_sec.map(|off| (now + off) as u64);

    let claims = json!({
        "iss": iss,
        "aud": aud,
        "sub": sub,
        "exp": exp,
        "nbf": nbf,
        "preferred_username": preferred_username,
        "name": name,
        "scope": scope,
    });

    encode(&header, &claims, encoding_key).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn generate_token_with_typ(
    pem: &str,
    kid: &str,
    typ: Option<&str>,
    iss: &str,
    aud: &str,
    sub: &str,
    preferred_username: Option<&str>,
    name: Option<&str>,
    scope: Option<&str>,
    exp_offset_sec: i64,
    nbf_offset_sec: Option<i64>,
) -> String {
    let enc_key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode_token(
        Algorithm::RS256,
        &enc_key,
        kid,
        typ,
        iss,
        aud,
        sub,
        preferred_username,
        name,
        scope,
        exp_offset_sec,
        nbf_offset_sec,
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_ed_token(
    kid: &str,
    typ: Option<&str>,
    iss: &str,
    aud: &str,
    sub: &str,
    preferred_username: Option<&str>,
    name: Option<&str>,
    scope: Option<&str>,
    exp_offset_sec: i64,
) -> String {
    let enc_key = EncodingKey::from_ed_pem(TEST_ED25519_PEM.as_bytes()).unwrap();
    encode_token(
        Algorithm::EdDSA,
        &enc_key,
        kid,
        typ,
        iss,
        aud,
        sub,
        preferred_username,
        name,
        scope,
        exp_offset_sec,
        None,
    )
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
    generate_token_with_typ(
        pem,
        kid,
        Some("at+jwt"),
        iss,
        aud,
        sub,
        preferred_username,
        name,
        scope,
        exp_offset_sec,
        None,
    )
}

async fn setup_test_app(
    mock_server: &MockServer,
    explicit_jwks: bool,
) -> (axum::Router, AppState, OidcConfig) {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let issuer = mock_server.uri();
    let audience = "zoo-client-test".to_string();
    let jwks_url = if explicit_jwks {
        Some(format!("{}/.well-known/jwks.json", mock_server.uri()))
    } else {
        None
    };

    let oidc_config = OidcConfig {
        issuer: issuer.clone(),
        audience: audience.clone(),
        jwks_url,
    };

    let mut config = Config::from_env();
    config.oidc = Some(oidc_config.clone());

    let db = init_db(&config.database_url).await.unwrap();
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    (app, state, oidc_config)
}

async fn fetch_current_user(app: &axum::Router, token: &str) -> anyhow::Result<Value> {
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[tokio::test]
async fn test_oidc_jit_provisioning_and_user_profile_flow() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, state, oidc_cfg) = setup_test_app(&mock_server, true).await;

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
        Some("openid profile"),
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

    // DB に user_identities が正しく登録されていることの検証 (users テーブルには IdP 識別子が保持されていないこと)
    let mut db = state.db.clone();
    let identities = UserIdentity::filter(
        UserIdentity::fields()
            .iss()
            .eq(&oidc_cfg.issuer)
            .and(UserIdentity::fields().sub().eq(&sub_1)),
    )
    .exec(&mut db)
    .await?;
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].user_id, user_id);

    // 2. 再ログインで同じ内部ユーザー ID に解決されること (キャッシュ経由)
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

    // 3. PATCH /api/users/me (プロフィールの本人限定更新と txid 返却)
    let patch_payload = json!({
        "display_name": "Alice The Graduate",
    });
    let req_patch = Request::builder()
        .method("PATCH")
        .uri("/api/users/me")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_vec(&patch_payload)?))?;

    let resp_patch = app.clone().oneshot(req_patch).await?;
    assert_eq!(resp_patch.status(), StatusCode::OK);
    let patch_body = to_bytes(resp_patch.into_body(), usize::MAX).await?;
    let patch_json: Value = serde_json::from_slice(&patch_body)?;
    assert_eq!(patch_json["data"]["display_name"], "Alice The Graduate");
    let txid = patch_json["txid"].as_str().expect("txid required");
    assert!(!txid.is_empty());

    // 4. GET /api/users/me で更新が反映されていること
    let req_get = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let resp_get = app.clone().oneshot(req_get).await?;
    let body_get = to_bytes(resp_get.into_body(), usize::MAX).await?;
    let json_get: Value = serde_json::from_slice(&body_get)?;
    assert_eq!(json_get["display_name"], "Alice The Graduate");

    Ok(())
}

#[tokio::test]
async fn test_preferred_username_spn_format() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    // Kanidm の SPN 形式 (name@realm.org)
    let sub = format!("spn-sub-{}", uuid::Uuid::new_v4().simple());
    let expected_name = format!(
        "kanidm_user_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let spn = format!("{expected_name}@idm.example.org");
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &sub,
        Some(&spn),
        Some("Kanidm User"),
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    // @ より前がユーザー名として採用されること
    assert_eq!(user_json["user_name"], expected_name);

    Ok(())
}

#[tokio::test]
async fn test_oidc_jit_username_contract_and_collision_suffix() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let unique = uuid::Uuid::new_v4().simple().to_string();
    let max_length_name = format!("{}{}", &unique[..12], "a".repeat(68));
    assert_eq!(max_length_name.chars().count(), 80);

    let first_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("username-first-{}", uuid::Uuid::new_v4().simple()),
        Some(&max_length_name),
        Some("First User"),
        Some("openid profile"),
        3600,
    );
    let first = fetch_current_user(&app, &first_token).await?;
    assert_eq!(first["user_name"], max_length_name);

    let second_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("username-second-{}", uuid::Uuid::new_v4().simple()),
        Some(&max_length_name),
        Some("Second User"),
        Some("openid profile"),
        3600,
    );
    let second = fetch_current_user(&app, &second_token).await?;
    assert_ne!(second["id"], first["id"]);

    let second_name = second["user_name"].as_str().unwrap();
    let prefix: String = max_length_name.chars().take(70).collect();
    assert!(second_name.starts_with(&format!("{prefix}_")));
    let suffix = &second_name[prefix.len() + 1..];
    assert_eq!(suffix.len(), 6);
    assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(second_name.chars().count(), 77);
    assert!(validate_user_name(second_name).is_ok());

    let reserved_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("username-reserved-{}", uuid::Uuid::new_v4().simple()),
        Some("admin"),
        Some("Reserved User"),
        Some("openid profile"),
        3600,
    );
    let reserved = fetch_current_user(&app, &reserved_token).await?;
    let reserved_name = reserved["user_name"].as_str().unwrap();
    assert_ne!(reserved_name, "admin");
    assert!(reserved_name.starts_with("user_"));
    assert_eq!(reserved_name.len(), 13);
    assert!(
        reserved_name["user_".len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    );
    assert!(validate_user_name(reserved_name).is_ok());

    Ok(())
}

#[tokio::test]
async fn test_rfc9068_typ_validation() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    // 1. typ ヘッダなし (ID トークンやレガシー JWT の場合) -> 401 Unauthorized
    let token_no_typ = generate_token_with_typ(
        TEST_KEY1_PEM,
        "test-key-1",
        None,
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-no-typ",
        Some("user_no_typ"),
        Some("No Typ"),
        None,
        3600,
        None,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token_no_typ}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2. typ: "JWT" (一般的な ID トークン) -> 401 Unauthorized
    let token_jwt_typ = generate_token_with_typ(
        TEST_KEY1_PEM,
        "test-key-1",
        Some("JWT"),
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-jwt-typ",
        Some("user_jwt_typ"),
        Some("JWT Typ"),
        None,
        3600,
        None,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token_jwt_typ}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. typ: "at+jwt" (RFC 9068 適合) -> 200 OK
    let token_at_jwt = generate_token_with_typ(
        TEST_KEY1_PEM,
        "test-key-1",
        Some("at+jwt"),
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-at-jwt",
        Some("user_at_jwt"),
        Some("At JWT"),
        None,
        3600,
        None,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token_at_jwt}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. typ: "application/at+jwt" (大文字小文字無視) -> 200 OK
    let token_app_at_jwt = generate_token_with_typ(
        TEST_KEY1_PEM,
        "test-key-1",
        Some("application/at+jwt"),
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-app-at-jwt",
        Some("user_app_at_jwt"),
        Some("App At JWT"),
        None,
        3600,
        None,
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token_app_at_jwt}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_openid_discovery_resolution() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    let custom_jwks_path = "/oauth2/openid/zoo-client/public_key.jwk";
    let discovery_body = json!({
        "issuer": mock_server.uri(),
        "jwks_uri": format!("{}{custom_jwks_path}", mock_server.uri())
    });

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&discovery_body))
        .mount(&mock_server)
        .await;

    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path(custom_jwks_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, false).await;
    assert!(oidc_cfg.jwks_url.is_none());

    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-discovery-user",
        Some("discovery_user"),
        Some("Discovery User"),
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_openid_discovery_issuer_mismatch_bad_gateway() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    // Discovery の issuer が設定と異なる悪意ある/誤設定のケース
    let discovery_body = json!({
        "issuer": "https://malicious-idp.example.com",
        "jwks_uri": format!("{}/jwks", mock_server.uri())
    });

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&discovery_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, false).await;

    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-mismatch",
        Some("user"),
        Some("User"),
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    // 上流 IdP の設定不一致は 502 Bad Gateway
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

    Ok(())
}

#[tokio::test]
async fn test_upstream_idp_outage_returns_bad_gateway() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    // Discovery エンドポイントが 500 エラーを返す
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, false).await;

    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-outage",
        Some("user"),
        Some("User"),
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    // IdP 障害は 401 ではなく 502 Bad Gateway
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

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

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

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

    // 3. nbf が未来 (nbf > now)
    let nbf_future_token = generate_token_with_typ(
        TEST_KEY1_PEM,
        "test-key-1",
        Some("at+jwt"),
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        "sub-nbf-future",
        Some("bob"),
        Some("Bob"),
        None,
        3600,
        Some(300), // 300秒未来
    );
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {nbf_future_token}"))
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 4. Issuer 不一致
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

    // 5. Audience 不一致
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

    Ok(())
}

#[tokio::test]
async fn test_oidc_key_rotation() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    let jwks_body = json!({ "keys": [key1_jwk(), key2_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

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
        None,
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

    Ok(())
}

#[tokio::test]
async fn test_eddsa_jwks_with_explicit_alg() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    // Better Auth OAuth Provider が発行する Ed25519 の JWKS を再現
    let jwks_body = json!({ "keys": [ed25519_jwk(true)] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let username = format!("eddsa_{}", &uuid::Uuid::new_v4().simple().to_string()[..6]);
    let token = generate_ed_token(
        "test-ed-key-1",
        Some("at+jwt"),
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("eddsa-sub-{}", uuid::Uuid::new_v4().simple()),
        Some(&username),
        Some("Ed DSA"),
        Some("openid profile"),
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    assert_eq!(user_json["user_name"], username);

    Ok(())
}

#[tokio::test]
async fn test_eddsa_jwk_without_alg_inferred_from_octet_key_pair() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;

    // alg を省略した OKP キーでも kty/crv から EdDSA と推論できること
    let jwks_body = json!({ "keys": [ed25519_jwk(false)] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let username = format!(
        "eddsa_noalg_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let token = generate_ed_token(
        "test-ed-key-1",
        Some("at+jwt"),
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("eddsa-noalg-sub-{}", uuid::Uuid::new_v4().simple()),
        Some(&username),
        Some("Ed DSA No Alg"),
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    assert_eq!(user_json["user_name"], username);

    Ok(())
}

#[tokio::test]
async fn test_production_environment_fail_closed_guards() -> anyhow::Result<()> {
    // 【重要】プロセス全体の環境変数を書き換えるのではなく、
    // Config の dev_auth_enabled = false / environment = "production" を直接注入してテスト。
    // 他の並行テストへの影響や flake を完全に排除。
    let mock_server = MockServer::start().await;
    let jwks_body = json!({ "keys": [key1_jwk()] });
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks_body))
        .mount(&mock_server)
        .await;

    let (_default_app, base_state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let mut prod_config = base_state.config.clone();
    prod_config.dev_auth_enabled = false;
    prod_config.environment = "production".to_string();
    assert!(prod_config.validate().is_ok());

    let prod_state = AppState::new(base_state.db.clone(), prod_config);
    let app = create_router(prod_state);

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

    // 3. 本番でヘッダなしは拒否 (GET /api/users/me)
    let req3 = Request::builder()
        .uri("/api/users/me")
        .body(Body::empty())?;
    let resp3 = app.clone().oneshot(req3).await?;
    assert_eq!(resp3.status(), StatusCode::UNAUTHORIZED);

    // 4. 本番で GET /api/users (ユーザー一覧) も未認証アクセス拒否
    let req_list = Request::builder().uri("/api/users").body(Body::empty())?;
    let resp_list = app.clone().oneshot(req_list).await?;
    assert_eq!(resp_list.status(), StatusCode::UNAUTHORIZED);

    // 5. 本番で GET /api/users/{id} (特定ユーザー情報) も未認証アクセス拒否
    let req_get_id = Request::builder()
        .uri(format!("/api/users/{test_uuid}"))
        .body(Body::empty())?;
    let resp_get_id = app.clone().oneshot(req_get_id).await?;
    assert_eq!(resp_get_id.status(), StatusCode::UNAUTHORIZED);

    // 6. 本番で POST /api/users (直接作成) は拒否
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

    // 7. 正規の OIDC Bearer Token なら本番でもアクセス成功
    let valid_token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("sub-prod-{}", uuid::Uuid::new_v4().simple()),
        Some("prod_user"),
        Some("Prod User"),
        None,
        3600,
    );
    let req5 = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {valid_token}"))
        .body(Body::empty())?;
    let resp5 = app.clone().oneshot(req5).await?;
    assert_eq!(resp5.status(), StatusCode::OK);

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

    let (app, state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let shared_sub = format!("concurrent-sub-{}", uuid::Uuid::new_v4().simple());
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &shared_sub,
        Some("concurrent_user"),
        Some("Concurrent User"),
        None,
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

    // user_identities テーブル内に (issuer, shared_sub) のレコードがちょうど 1 件だけ存在することを確認
    let mut db = state.db.clone();
    let identities = UserIdentity::filter(
        UserIdentity::fields()
            .iss()
            .eq(&oidc_cfg.issuer)
            .and(UserIdentity::fields().sub().eq(&shared_sub)),
    )
    .exec(&mut db)
    .await?;

    assert_eq!(
        identities.len(),
        1,
        "Exactly one user identity should be provisioned"
    );

    // users テーブルにも対応するユーザーがちょうど 1 人だけ存在することを確認
    let users = User::filter(User::fields().id().eq(&identities[0].user_id))
        .exec(&mut db)
        .await?;
    assert_eq!(users.len(), 1, "Exactly one user record should exist");

    Ok(())
}

fn discovery_body(mock_server: &MockServer, jwks_path: &str, userinfo_path: &str) -> Value {
    json!({
        "issuer": mock_server.uri(),
        "jwks_uri": format!("{}{}", mock_server.uri(), jwks_path),
        "userinfo_endpoint": format!("{}{}", mock_server.uri(), userinfo_path),
    })
}

async fn mount_jwks(mock_server: &MockServer, jwks_path: &str) {
    Mock::given(method("GET"))
        .and(path(jwks_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [key1_jwk()] })))
        .mount(mock_server)
        .await;
}

async fn mount_discovery(mock_server: &MockServer, jwks_path: &str, userinfo_path: &str) {
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(discovery_body(
            mock_server,
            jwks_path,
            userinfo_path,
        )))
        .mount(mock_server)
        .await;
}

#[tokio::test]
async fn test_userinfo_provides_jit_profile_names() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_path = "/.well-known/jwks.json";
    let userinfo_path = "/oauth2/openid/zoo-client/userinfo";

    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, false).await;

    let sub = format!("userinfo-sub-{}", uuid::Uuid::new_v4().simple());
    let username = format!(
        "authz_user_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &sub,
        None,
        None,
        Some("openid profile"),
        3600,
    );

    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .and(header("authorization", format!("Bearer {token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sub": &sub,
            "preferred_username": &username,
            "name": "AuthZ Profile User",
        })))
        .mount(&mock_server)
        .await;

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    assert_eq!(user_json["user_name"], username);
    assert_eq!(user_json["display_name"], "AuthZ Profile User");

    Ok(())
}

#[tokio::test]
async fn test_userinfo_failure_falls_back_to_user() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_path = "/.well-known/jwks.json";
    let userinfo_path = "/oauth2/openid/zoo-client/userinfo";

    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("userinfo-failure-sub-{}", uuid::Uuid::new_v4().simple()),
        None,
        None,
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    // UserInfo が失敗してもログインは継続し、従来のフォールバックが使われる
    assert_eq!(user_json["display_name"], "user");
    assert!(
        user_json["user_name"]
            .as_str()
            .is_some_and(|name| name.starts_with("user"))
    );

    Ok(())
}

#[tokio::test]
async fn test_userinfo_subject_mismatch_falls_back_to_user() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_path = "/.well-known/jwks.json";
    let userinfo_path = "/oauth2/openid/zoo-client/userinfo";

    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sub": "different-subject",
            "preferred_username": "mallory",
            "name": "Mallory",
        })))
        .mount(&mock_server)
        .await;

    let (app, _state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &format!("userinfo-mismatch-sub-{}", uuid::Uuid::new_v4().simple()),
        None,
        None,
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    // sub が一致しない UserInfo は採用せず、フォールバックする
    assert_eq!(user_json["display_name"], "user");

    Ok(())
}

#[tokio::test]
async fn test_legacy_display_name_repaired_once_without_overwriting_edits() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_path = "/.well-known/jwks.json";
    let userinfo_path = "/oauth2/openid/zoo-client/userinfo";

    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let (app, state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let sub = format!("legacy-sub-{}", uuid::Uuid::new_v4().simple());
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &sub,
        None,
        None,
        None,
        3600,
    );

    // 1. UserInfo 障害時に旧フォールバックでユーザーが作成される
    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    let user_id = user_json["id"]
        .as_str()
        .expect("id must be string")
        .to_string();
    assert_eq!(user_json["display_name"], "user");

    // 2. 再ログイン時に UserInfo が利用できると表示名が補正される
    mock_server.reset().await;
    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sub": &sub,
            "preferred_username": "legacy_user",
            "name": "Recovered User",
        })))
        .mount(&mock_server)
        .await;

    let state2 = AppState::new(state.db.clone(), state.config.clone());
    let app2 = create_router(state2);
    let req2 = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let resp2 = app2.oneshot(req2).await?;
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = to_bytes(resp2.into_body(), usize::MAX).await?;
    let user_json2: Value = serde_json::from_slice(&body2)?;
    assert_eq!(user_json2["id"].as_str(), Some(user_id.as_str()));
    assert_eq!(user_json2["display_name"], "Recovered User");

    // 3. 補正後は UserInfo の名前が変わっても上書きしない (ユーザー編集との競合回避)
    mock_server.reset().await;
    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sub": &sub,
            "preferred_username": "legacy_user",
            "name": "Renamed At IdP",
        })))
        .mount(&mock_server)
        .await;

    let state3 = AppState::new(state.db.clone(), state.config.clone());
    let app3 = create_router(state3);
    let req3 = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let resp3 = app3.oneshot(req3).await?;
    assert_eq!(resp3.status(), StatusCode::OK);
    let body3 = to_bytes(resp3.into_body(), usize::MAX).await?;
    let user_json3: Value = serde_json::from_slice(&body3)?;
    assert_eq!(user_json3["display_name"], "Recovered User");

    Ok(())
}

#[tokio::test]
async fn test_display_name_repair_does_not_overwrite_concurrent_user_edit() -> anyhow::Result<()> {
    let mock_server = MockServer::start().await;
    let jwks_path = "/.well-known/jwks.json";
    let userinfo_path = "/oauth2/openid/zoo-client/userinfo";

    // 1. UserInfo 障害時に旧フォールバックでユーザーを作成する
    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let (app, state, oidc_cfg) = setup_test_app(&mock_server, true).await;

    let sub = format!("concurrent-edit-sub-{}", uuid::Uuid::new_v4().simple());
    let token = generate_token(
        TEST_KEY1_PEM,
        "test-key-1",
        &oidc_cfg.issuer,
        &oidc_cfg.audience,
        &sub,
        None,
        None,
        None,
        3600,
    );

    let req = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let user_json: Value = serde_json::from_slice(&body)?;
    let user_id = user_json["id"]
        .as_str()
        .expect("id must be string")
        .to_string();
    assert_eq!(user_json["display_name"], "user");

    // 2. UserInfo の応答を遅延させ、その間に本人の表示名編集を模擬する
    mock_server.reset().await;
    mount_jwks(&mock_server, jwks_path).await;
    mount_discovery(&mock_server, jwks_path, userinfo_path).await;
    Mock::given(method("GET"))
        .and(path(userinfo_path))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(500))
                .set_body_json(json!({
                    "sub": &sub,
                    "preferred_username": "legacy_user",
                    "name": "Recovered User",
                })),
        )
        .mount(&mock_server)
        .await;

    let state2 = AppState::new(state.db.clone(), state.config.clone());
    let app2 = create_router(state2);
    let req2 = Request::builder()
        .uri("/api/users/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let handle = tokio::spawn(async move { app2.oneshot(req2).await });

    // UserInfo へのリクエストが届くまで待つ (届いた時点では応答は保留中)
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let received = mock_server.received_requests().await.unwrap_or_default();
        if received
            .iter()
            .any(|request| request.url.path() == userinfo_path)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "UserInfo request was not observed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let now = backend::models::now_utc_millis();
    User::update_by_id(user_id.as_str())
        .display_name("Edited By User")
        .updated_at(Some(now))
        .exec(&mut tx)
        .await?;
    tx.commit().await?;

    let resp2 = handle.await??;
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = to_bytes(resp2.into_body(), usize::MAX).await?;
    let user_json2: Value = serde_json::from_slice(&body2)?;
    // UserInfo の応答が遅れている間に本人が編集した表示名を上書きしない
    assert_eq!(user_json2["display_name"], "Edited By User");

    let mut db = state.db.clone();
    let stored_user = User::get_by_id(&mut db, user_id.as_str()).await?;
    assert_eq!(stored_user.display_name, "Edited By User");

    Ok(())
}
