use super::*;
use crate::auth::AuthUser;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
fn response(value: Value) -> Response {
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], Json(value)).into_response()
}
#[derive(Default, Deserialize)]
struct Page {
    page: Option<bool>,
    before: Option<String>,
}
#[derive(Deserialize)]
struct Resource {
    resource: String,
}
#[derive(Deserialize)]
struct FollowRequest {
    actor: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(interactions::router())
        .route("/.well-known/webfinger", get(webfinger))
        .route("/api/activitypub/users/{id}", get(actor))
        .route("/api/activitypub/users/{id}/outbox", get(outbox))
        .route("/api/activitypub/users/{id}/followers", get(followers))
        .route("/api/activitypub/users/{id}/following", get(following))
        .route(
            "/api/activitypub/users/{id}/inbox",
            post(inbox::receive).layer(DefaultBodyLimit::max(256 * 1024)),
        )
        .route("/api/activitypub/notes/{id}", get(note))
        .route("/api/activitypub/notes/{id}/likes", get(likes))
        .route(
            "/api/activitypub/follows",
            get(follows).post(follow).delete(unfollow),
        )
        .route("/api/activitypub/status", get(status))
        .route("/api/activitypub/search", get(search))
}
async fn local_user(state: &AppState, id: &str) -> Result<Value, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(&mut tx,"SELECT json_build_object('id',id,'user_name',user_name,'display_name',display_name)::text FROM users WHERE id=$1 AND NOT is_bot",&[id]).await
}
async fn actor(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    base_url()?;
    let user = local_user(&state, &id).await?;
    let key = keys(&state, &id).await?;
    let actor = actor_id(&id)?;
    Ok(response(
        json!({"@context":[CONTEXT,"https://w3id.org/security/v1",{"manuallyApprovesFollowers":"as:manuallyApprovesFollowers"}],"type":"Person","id":actor,"preferredUsername":user["user_name"],"name":user["display_name"],"inbox":format!("{actor}/inbox"),"outbox":format!("{actor}/outbox"),"followers":format!("{actor}/followers"),"following":format!("{actor}/following"),"manuallyApprovesFollowers":true,"publicKey":{"id":format!("{actor}#main-key"),"owner":actor,"publicKeyPem":key["public_key"]}}),
    ))
}
async fn webfinger(
    State(state): State<AppState>,
    Query(q): Query<Resource>,
) -> Result<Response, AppError> {
    let base = base_url()?;
    let url = reqwest::Url::parse(&base).map_err(internal)?;
    let account = q
        .resource
        .strip_prefix("acct:")
        .and_then(|s| s.rsplit_once('@'))
        .ok_or_else(|| AppError::BadRequest("Invalid acct resource".into()))?;
    if account.1 != url.authority() {
        return Err(AppError::NotFound("Account".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let u = one(
        &mut tx,
        "SELECT json_build_object('id',id)::text FROM users WHERE user_name=$1 AND NOT is_bot",
        &[account.0],
    )
    .await?;
    Ok(([(header::CONTENT_TYPE,"application/jrd+json")],Json(json!({"subject":q.resource,"links":[{"rel":"self","type":CONTENT_TYPE,"href":actor_id(string(&u,"id")?)?}]}))).into_response())
}
async fn note(State(state): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    base_url()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let post=one(&mut tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1 AND apub AND visibility IN ('public','unlisted')",&[&id]).await?;
    if !post["deleted_at"].is_null() {
        return Ok((StatusCode::GONE,response(json!({"@context":CONTEXT,"type":"Tombstone","id":format!("{}/api/activitypub/notes/{id}",base_url()?)}))).into_response());
    }
    Ok(response(note_value(&post)?))
}
async fn outbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Response, AppError> {
    local_user(&state, &id).await?;
    let url = format!("{}/outbox", actor_id(&id)?);
    if page.page != Some(true) {
        return Ok(response(
            json!({"@context":CONTEXT,"type":"OrderedCollection","id":url,"first":format!("{url}?page=true")}),
        ));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE user_id=$1 AND apub AND visibility='public' AND deleted_at IS NULL AND ($2='' OR (created_at,id)<(SELECT created_at,id FROM home_posts WHERE id=$2)) ORDER BY created_at DESC,id DESC LIMIT 50",&[&id,page.before.as_deref().unwrap_or("")]).await?;
    let items=rows.iter().map(|p|{let note=note_value(p)?;Ok(json!({"type":"Create","id":format!("{}#create",string(&note,"id")?),"actor":actor_id(&id)?,"published":p["created_at"],"to":note["to"],"cc":note["cc"],"object":note}))}).collect::<Result<Vec<Value>,AppError>>()?;
    let mut result = json!({"@context":CONTEXT,"type":"OrderedCollectionPage","id":format!("{url}?page=true{}",page.before.as_ref().map(|s|format!("&before={}",percent_encoding::utf8_percent_encode(s,percent_encoding::NON_ALPHANUMERIC))).unwrap_or_default()),"partOf":url,"orderedItems":items});
    if rows.len() == 50 {
        result["next"] = json!(format!(
            "{url}?page=true&before={}",
            string(rows.last().unwrap(), "id")?
        ));
    }
    Ok(response(result))
}
async fn followers(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    collection(&s, &id, "incoming", "followers").await
}
async fn following(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    collection(&s, &id, "outgoing", "following").await
}
async fn collection(
    state: &AppState,
    user: &str,
    direction: &str,
    name: &str,
) -> Result<Response, AppError> {
    local_user(state, user).await?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let count=one(&mut tx,"SELECT json_build_object('count',count(*))::text FROM apub_follows WHERE user_id=$1 AND direction=$2 AND accepted",&[user,direction]).await?;
    // Do not expose follower identities. Collection URL remains the addressing target.
    Ok(response(
        json!({"@context":CONTEXT,"id":format!("{}/{name}",actor_id(user)?),"type":"OrderedCollection","totalItems":count["count"]}),
    ))
}
async fn follows(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(json!(query(&mut tx,"SELECT json_build_object('actor_id',f.actor_id,'activity_id',f.activity_id,'accepted',f.accepted,'direction',f.direction,'user_name',a.user_name,'display_name',a.display_name)::text FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id WHERE f.user_id=$1 ORDER BY a.user_name LIMIT 500",&[&user.id]).await?)))
}
async fn resolve(state: &AppState, user: &str, input: &str) -> Result<Value, AppError> {
    if input.len() > 2048 || input.contains('\0') {
        return Err(AppError::BadRequest("Invalid account length".into()));
    }
    let id = if input.starts_with("https://") {
        input.to_owned()
    } else {
        let (name, host) = input
            .trim_start_matches('@')
            .rsplit_once('@')
            .ok_or_else(|| {
                AppError::BadRequest("Enter @user@server or an actor HTTPS URL".into())
            })?;
        if name.is_empty() || name.len() > 200 || host.contains(['/', ':', '?', '#']) {
            return Err(AppError::BadRequest("Invalid remote account".into()));
        }
        let resource = format!("acct:{name}@{host}");
        let encoded =
            percent_encoding::utf8_percent_encode(&resource, percent_encoding::NON_ALPHANUMERIC);
        let doc = transport::fetch_json(
            state,
            Some(user),
            &format!("https://{host}/.well-known/webfinger?resource={encoded}"),
        )
        .await?;
        doc["links"].as_array().and_then(|links|links.iter().find(|v|v["rel"]=="self" && matches!(v["type"].as_str(),Some("application/activity+json"|"application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"")))).and_then(|v|v["href"].as_str()).ok_or_else(||AppError::BadGateway("No ActivityPub actor found".into()))?.to_owned()
    };
    remote_actor(state, Some(user), &id, false).await
}
async fn follow(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<FollowRequest>,
) -> Result<Json<Value>, AppError> {
    base_url()?;
    keys(&state, &user.id).await?;
    let remote = resolve(&state, &user.id, &body.actor).await?;
    let remote_id = string(&remote, "id")?;
    let local = actor_id(&user.id)?;
    let id = format!(
        "{}/api/activitypub/activities/{}",
        base_url()?,
        uuid::Uuid::new_v4()
    );
    let follow = json!({"@context":CONTEXT,"id":id,"type":"Follow","actor":local,"object":remote_id,"to":[remote_id]});
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let inserted=query(&mut tx,"INSERT INTO apub_follows(user_id,actor_id,direction,activity_id) VALUES($1,$2,'outgoing',$3) ON CONFLICT DO NOTHING RETURNING '{}'::text",&[&user.id,remote_id,&id]).await?;
    if !inserted.is_empty() {
        queue(&mut tx, &user.id, remote_id, &follow).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"actor_id":remote_id})))
}
async fn unfollow(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<FollowRequest>,
) -> Result<StatusCode, AppError> {
    let local = actor_id(&user.id)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"DELETE FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' RETURNING row_to_json(apub_follows)::text",&[&user.id,&body.actor]).await?;
    if let Some(f) = rows.first() {
        queue(&mut tx,&user.id,&body.actor,&json!({"@context":CONTEXT,"id":format!("{}/api/activitypub/activities/{}",base_url()?,uuid::Uuid::new_v4()),"type":"Undo","actor":local,"object":{"id":f["activity_id"],"type":"Follow","actor":local,"object":body.actor},"to":[body.actor]})).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn status(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let configured = base_url().is_ok();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let counts=one(&mut tx,"SELECT json_build_object('pending',count(*) FILTER(WHERE delivered_at IS NULL AND NOT failed),'failed',count(*) FILTER(WHERE failed))::text FROM apub_deliveries WHERE user_id=$1",&[&user.id]).await?;
    Ok(Json(
        json!({"enabled":configured,"actor":if configured{Some(actor_id(&user.id)?)}else{None},"delivery":counts}),
    ))
}

#[derive(Deserialize)]
struct Search {
    q: String,
}
async fn search(
    State(state): State<AppState>,
    user: AuthUser,
    Query(input): Query<Search>,
) -> Result<Json<Vec<Value>>, AppError> {
    let term = input.q.trim();
    if term.is_empty() || term.len() > 2048 {
        return Err(AppError::BadRequest("Invalid search".into()));
    }
    if term.starts_with("https://") || term.trim_start_matches('@').contains('@') {
        base_url()?;
        keys(&state, &user.id).await?;
        let remote = resolve(&state, &user.id, term).await?;
        return Ok(Json(vec![
            json!({"id":remote["id"],"user_name":remote["user_name"],"display_name":remote["display_name"],"remote":true,"url":remote["id"]}),
        ]));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, "SELECT json_build_object('id',id,'user_name',user_name,'display_name',display_name,'created_at',created_at,'remote',false)::text FROM users WHERE NOT is_bot AND (strpos(lower(user_name),lower($1))>0 OR strpos(lower(display_name),lower($1))>0) ORDER BY user_name LIMIT 30", &[term.trim_start_matches('@')]).await?;
    Ok(Json(rows))
}

async fn likes(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(&mut tx,"SELECT '{}'::text FROM home_posts WHERE id=$1 AND apub AND deleted_at IS NULL AND visibility IN ('public','unlisted')",&[&id]).await?;
    let count = one(
        &mut tx,
        "SELECT json_build_object('count',count(*))::text FROM apub_reactions WHERE post_id=$1",
        &[&id],
    )
    .await?;
    Ok(response(
        json!({"@context":CONTEXT,"id":format!("{}/api/activitypub/notes/{id}/likes",base_url()?),"type":"Collection","totalItems":count["count"]}),
    ))
}
