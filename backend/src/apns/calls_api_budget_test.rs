use crate::{
    auth::oidc::JwksVerifier,
    config::{Config, OidcConfig},
    state::AppState,
};
use futures_util::future::try_join_all;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header};
use openssl::{
    ec::{EcGroup, EcKey},
    nid::Nid,
    pkey::PKey,
};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Instant};

pub(super) async fn measure(url: &str, schema: &str) -> anyhow::Result<Value> {
    let mut results = Vec::new();
    for plan_mode in ["force_custom_plan", "force_generic_plan"] {
        for workers in [1, 8] {
            results.push(measure_group(url, schema, plan_mode, workers).await?);
        }
    }
    Ok(json!(results))
}

async fn measure_group(
    url: &str,
    schema: &str,
    plan_mode: &str,
    workers: usize,
) -> anyhow::Result<Value> {
    let at = Instant::now();
    let mut database_url = reqwest::Url::parse(url)?;
    database_url.query_pairs_mut().append_pair(
        "options",
        &format!("-csearch_path={schema} -cjit=off -cplan_cache_mode={plan_mode}"),
    );
    let mut config = Config::from_env();
    config.database_url = database_url.to_string();
    config.dev_auth_enabled = false;
    config.oidc = Some(OidcConfig {
        issuer: "https://incoming.example.invalid".into(),
        audience: "incoming-budget".into(),
        jwks_url: None,
        reauth_audiences: vec![],
    });
    // 実在する鍵を使わず、署名検証は本番と同じ経路を通す。JWKSの外部取得は測定しない。
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
    let key = PKey::from_ec_key(EcKey::generate(&group)?)?;
    let signing = EncodingKey::from_ec_pem(&key.private_key_to_pem_pkcs8()?)?;
    let verifier = JwksVerifier::with_keys(HashMap::from([(
        "incoming-budget".into(),
        (
            DecodingKey::from_ec_pem(&key.public_key_to_pem()?)?,
            Algorithm::ES256,
        ),
    )]));
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("incoming-budget".into());
    header.typ = Some("at+jwt".into());
    let mut tokens = Vec::new();
    for user in ["u1", "u2"] {
        tokens.push(jsonwebtoken::encode(
            &header,
            &json!({
                "iss":"https://incoming.example.invalid", "aud":"incoming-budget", "sub":user,
                "exp":jiff::Timestamp::now().as_second()+3600,
            }),
            &signing,
        )?);
    }
    let db = crate::db::init_db(&config.database_url).await?;
    // 条件ごとにLISTEN・sweeperを増やさず、HTTPと認証・DBプールだけを測定する。
    let mut state = AppState::new_for_worker(db, config);
    state.jwks_verifier = verifier;
    let app = crate::routes::create_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let preparation_ms = at.elapsed().as_secs_f64() * 1000.0;
    let result = async {
        let mut tasks = Vec::new();
        for worker in 0..workers {
            let base = base.clone();
            let tokens = tokens.clone();
            tasks.push(async move {
                let client = reqwest::Client::builder().no_proxy().build()?;
                let mut samples = Vec::new();
                for (name, id, user, pending) in [
                    ("招待状態", "c4ca4238-a0b9-2382-0dcc-509a6f75849b", 0, true),
                    (
                        "別人の招待",
                        "c4ca4238-a0b9-2382-0dcc-509a6f75849b",
                        1,
                        false,
                    ),
                    (
                        "存在しない招待",
                        "00000000-0000-0000-0000-000000000000",
                        0,
                        false,
                    ),
                    (
                        "取消済みの招待",
                        "c81e728d-9d4c-2f63-6f06-7f89cc14862c",
                        1,
                        false,
                    ),
                ] {
                    for attempt in 0..30 {
                        let at = Instant::now();
                        let response = client
                            .get(format!("{base}/api/call-invites/{id}"))
                            .bearer_auth(&tokens[user])
                            .send()
                            .await?;
                        let status = response.status();
                        let bytes = response.bytes().await?;
                        let elapsed_ms = at.elapsed().as_secs_f64() * 1000.0;
                        anyhow::ensure!(status == reqwest::StatusCode::OK, "{name}: HTTP {status}");
                        let body: Value = serde_json::from_slice(&bytes)?;
                        anyhow::ensure!(
                            body == json!({"pending":pending}),
                            "{name}: 応答が一致しません"
                        );
                        samples.push(
                            json!({"条件":name,"接続":worker,"回":attempt,"往復ms":elapsed_ms}),
                        );
                    }
                }
                Ok::<_, anyhow::Error>(samples)
            });
        }
        Ok::<_, anyhow::Error>(
            try_join_all(tasks)
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )
    }
    .await;
    // エラーでもHTTPサーバーと接続プールを閉じてから測定用schemaを回収する。
    let _ = stop.send(());
    let shutdown = server.await;
    let samples = result?;
    shutdown??;
    let mut summary = Vec::new();
    for name in ["招待状態", "別人の招待", "存在しない招待", "取消済みの招待"]
    {
        let mut values: Vec<f64> = samples
            .iter()
            .filter(|v| v["条件"] == name)
            .map(|v| v["往復ms"].as_f64().unwrap())
            .collect();
        values.sort_by(f64::total_cmp);
        let percentile = |p: f64| values[(values.len() as f64 * p).ceil() as usize - 1];
        summary.push(json!({"条件":name,"件数":values.len(),"p50":percentile(0.5),"p95":percentile(0.95),"最大":values.last()}));
    }
    Ok(
        json!({"接続数":workers,"計画方式":plan_mode,"準備ms":preparation_ms,
        "認証":"ES256署名・iss・aud・期限を検証。JWKS鍵は事前キャッシュ、利用者解決キャッシュは初期状態から計測",
        "DBプール上限":std::env::var("DB_MAX_POOL_SIZE").unwrap_or(crate::db::DEFAULT_MAX_POOL_SIZE.to_string()),
        "集計":summary,"測定":samples}),
    )
}
