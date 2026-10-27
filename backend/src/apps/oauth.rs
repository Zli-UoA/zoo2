use super::*;
use axum::{Json, Router, extract::State, middleware, routing::post};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/oauth/authorize", post(authorize))
        .route("/api/apps/oauth/token", post(token))
        .route("/api/apps/oauth/revoke", post(revoke))
        .route_layer(middleware::from_fn(
            |r: axum::extract::Request, n: middleware::Next| async move {
                let mut response = n.run(r).await;
                response.headers_mut().insert(
                    "cache-control",
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorize {
    app_id: String,
    redirect_uri: String,
    code_challenge: String,
    #[serde(default)]
    scopes: Vec<String>,
    state: String,
}
async fn authorize(
    State(s): State<AppState>,
    actor: AuthUser,
    Json(b): Json<Authorize>,
) -> Result<Json<Value>, AppError> {
    if b.code_challenge.len() != 43
        || !b
            .code_challenge
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || b.state.len() > 1024
    {
        return Err(AppError::BadRequest(
            "An S256 PKCE challenge and state are required".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let i=one(&mut tx,"SELECT json_build_object('id',i.id,'manifest',i.manifest::jsonb,'scopes',i.scopes::jsonb)::text FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND a.status='installed' AND i.active",&[&b.app_id]).await?;
    let manifest: Manifest =
        serde_json::from_value(i["manifest"].clone()).map_err(|e| AppError::Internal(e.into()))?;
    if !manifest.redirect_uris.contains(&b.redirect_uri)
        || b.scopes
            .iter()
            .any(|scope| !manifest.scopes.contains(scope))
    {
        return Err(AppError::BadRequest(
            "Unregistered redirect URI or unapproved scope".into(),
        ));
    }
    let code = secret("zoo_code_");
    toasty::sql::statement("INSERT INTO app_oauth_codes(code_hash,installation_id,user_id,redirect_uri,code_challenge,scopes,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+interval '5 minutes')")
 .bind(hash(&code)).bind(i["id"].as_str().unwrap()).bind(actor.id.as_str()).bind(b.redirect_uri.as_str()).bind(b.code_challenge).bind(serde_json::to_string(&b.scopes).unwrap()).exec(&mut tx).await?;
    audit(&mut tx, &b.app_id, &actor.id, "user.consented").await?;
    tx.commit().await?;
    let mut uri = reqwest::Url::parse(&b.redirect_uri).map_err(|e| AppError::Internal(e.into()))?;
    uri.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &b.state);
    Ok(Json(json!({"redirect_uri":uri.as_str()})))
}
#[derive(Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case")]
enum TokenRequest {
    AuthorizationCode {
        code: String,
        redirect_uri: String,
        code_verifier: String,
    },
    RefreshToken {
        refresh_token: String,
    },
}
async fn token(
    State(s): State<AppState>,
    Json(b): Json<TokenRequest>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let row=match b{
 TokenRequest::AuthorizationCode{code,redirect_uri,code_verifier}=>{
  if !(43..=128).contains(&code_verifier.len())||!code_verifier.bytes().all(|c|c.is_ascii_alphanumeric()||b"-._~".contains(&c)){return Err(AppError::BadRequest("Invalid PKCE verifier".into()));}
  let challenge=URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
  one(&mut tx,"UPDATE app_oauth_codes c SET used_at=now() FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE c.installation_id=i.id AND c.code_hash=$1 AND c.redirect_uri=$2 AND c.code_challenge=$3 AND c.used_at IS NULL AND c.expires_at>now() AND i.active AND a.status='installed' RETURNING json_build_object('installation',c.installation_id,'user',c.user_id,'scopes',c.scopes::jsonb)::text",&[&hash(&code),&redirect_uri,&challenge]).await
 },
 TokenRequest::RefreshToken{refresh_token}=>one(&mut tx,"UPDATE app_credentials t SET revoked_at=now() FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE t.installation_id=i.id AND t.refresh_hash=$1 AND t.revoked_at IS NULL AND t.created_at>now()-interval '30 days' AND i.active AND a.status='installed' RETURNING json_build_object('installation',t.installation_id,'user',t.user_id,'scopes',t.scopes::jsonb)::text",&[&hash(&refresh_token)]).await
 }.map_err(|e|match e{AppError::NotFound(_)=>AppError::Unauthorized("Invalid or consumed OAuth grant".into()),other=>other})?;
    let value = mint(
        &mut tx,
        row["installation"].as_str().unwrap(),
        row["user"].as_str(),
        &row["scopes"],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}
#[derive(Deserialize)]
struct Revoke {
    token: String,
}
async fn revoke(State(s): State<AppState>, Json(b): Json<Revoke>) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "UPDATE app_credentials SET revoked_at=now() WHERE token_hash=$1 OR refresh_hash=$1",
    )
    .bind(hash(&b.token))
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
