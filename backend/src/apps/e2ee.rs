//! Bot の E2EE の端末の API（#366。仕様は `docs/e2ee.md` の「Bot の端末」）。
//!
//! Bot（App のボットユーザー）は、人間の端末と同じ API（`/api/e2ee/...`）を、App のトークンで
//! `/api/apps/v1/e2ee/...` から呼ぶ。中身は人間の API の処理をそのまま使い、呼び出し元をボットユーザーにする。
//! 違いは次のとおり。
//!
//! - 委任でない、`e2ee:device` を持つボットのトークンだけが使える（委任トークンはユーザーとして動くので、
//!   ユーザーの端末を作らせない）。
//! - Bot は Electric の Shape を使わないので、to-device は `GET /api/apps/v1/e2ee/to_device` で受け取る。
//! - クロス署名鍵の登録に直前の再認証を求めない（`CrossSigningAuth::BotDevice`）。
//! - 配布先のスナップショット（`recipients`）は、メンバーが許可して参加したチャンネルだけ。
//!
//! 暗号化したメッセージの送信、編集、削除、リアクション、退出は、今までの `/api/apps/v1/messages` などを使う
//! （`Principal::e2ee_channel`）。Secret Storage と鍵バックアップは Bot には出さない（鍵は Bot の手元の
//! 暗号ストアにだけ置く）。

use super::*;
use crate::routes::{
    e2ee as keys, e2ee::to_device, e2ee_channels, e2ee_cross_signing as cross_signing,
    response::ApiResponse,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::HeaderMap,
    response::Response,
    routing::{get, post},
};

/// `GET /api/apps/v1/e2ee/to_device` で 1 回に返すメッセージの上限。
pub const MAX_TO_DEVICE_PAGE: i64 = 100;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/apps/v1/e2ee/devices",
            get(list_devices).post(register_device),
        )
        .route(
            "/api/apps/v1/e2ee/devices/{device_id}",
            axum::routing::delete(delete_device),
        )
        .route("/api/apps/v1/e2ee/keys/upload", post(upload_keys))
        .route("/api/apps/v1/e2ee/keys/query", post(query_keys))
        .route("/api/apps/v1/e2ee/keys/claim", post(claim_keys))
        .route(
            "/api/apps/v1/e2ee/to_device",
            get(receive_to_device)
                .post(send_to_device)
                .layer(DefaultBodyLimit::max(to_device::TO_DEVICE_BODY_LIMIT)),
        )
        .route("/api/apps/v1/e2ee/to_device/ack", post(ack_to_device))
        .route(
            "/api/apps/v1/e2ee/cross_signing/keys",
            post(upload_cross_signing_keys),
        )
        .route(
            "/api/apps/v1/e2ee/cross_signing/signatures",
            post(upload_signatures),
        )
        .route(
            "/api/apps/v1/e2ee/tofu_pins",
            get(list_pins).post(upload_pins),
        )
        .route(
            "/api/apps/v1/e2ee/channels/{channel_id}/recipients",
            get(get_recipients),
        )
        .layer(axum::middleware::map_response(super::no_store))
}

/// Bot の端末として動けるトークンか確かめ、ボットユーザーを返す。
fn bot_device(p: &Principal) -> Result<AuthUser, AppError> {
    if p.delegated {
        return Err(AppError::Forbidden(
            "Delegated tokens cannot use E2EE devices".into(),
        ));
    }
    p.require(E2EE_DEVICE_SCOPE)?;
    debug_assert!(p.is_bot_device());
    Ok(p.actor())
}

async fn register_device(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<keys::RegisterRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    keys::register_device(State(state), bot_device(&p)?, Json(req)).await
}

async fn list_devices(
    State(state): State<AppState>,
    p: Principal,
) -> Result<Json<Vec<Value>>, AppError> {
    keys::list_devices(State(state), bot_device(&p)?).await
}

/// Bot が自分の端末を消す（鍵の保存先を失った、作り直す、など）。消すと、Bot が参加している暗号化チャンネルの
/// epoch が増え、それ以降のセッションはその端末に配られない。
async fn delete_device(
    State(state): State<AppState>,
    p: Principal,
    Path(device_id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    keys::delete_device(State(state), bot_device(&p)?, Path(device_id)).await
}

async fn upload_keys(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<keys::UploadRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    keys::upload_keys(State(state), bot_device(&p)?, Json(req)).await
}

async fn query_keys(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<keys::QueryRequest>,
) -> Result<Response, AppError> {
    keys::query_keys(State(state), bot_device(&p)?, Json(req)).await
}

async fn claim_keys(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<keys::ClaimRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    keys::claim_keys(State(state), bot_device(&p)?, Json(req)).await
}

async fn send_to_device(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<to_device::SendRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    to_device::send_to_device(State(state), bot_device(&p)?, Json(req)).await
}

#[derive(Deserialize)]
struct ReceiveQuery {
    device_id: String,
    #[serde(default)]
    limit: Option<i64>,
}

/// `GET /api/apps/v1/e2ee/to_device?device_id=...`：自分の端末あての、受信確認していない to-device メッセージを
/// 古い順に返す。人間の端末の Shape（`/api/shapes/e2ee_to_device`）と同じ列。処理したら `to_device/ack` で消す。
async fn receive_to_device(
    State(state): State<AppState>,
    p: Principal,
    Query(q): Query<ReceiveQuery>,
) -> Result<Json<Value>, AppError> {
    let actor = bot_device(&p)?;
    let limit = q.limit.unwrap_or(MAX_TO_DEVICE_PAGE);
    if !(1..=MAX_TO_DEVICE_PAGE).contains(&limit) {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {MAX_TO_DEVICE_PAGE}"
        )));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if !to_device::own_active_device(&mut tx, &actor.id, &q.device_id).await? {
        return Err(crate::routes::e2ee::device_not_found());
    }
    let messages = query(
        &mut tx,
        "SELECT json_build_object('id', id::text, 'sender_user_id', sender_user_id,
           'sender_device_id', sender_device_id, 'type', type, 'content', content)::text
         FROM e2ee_to_device_messages
         WHERE recipient_user_id = $1 AND recipient_device_id = $2
         ORDER BY id LIMIT $3::bigint",
        &[&actor.id, &q.device_id, &limit.to_string()],
    )
    .await?;
    keys::touch_last_seen(&mut tx, &actor.id, &q.device_id).await?;
    tx.commit().await?;
    Ok(Json(json!({ "messages": messages })))
}

async fn ack_to_device(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<to_device::AckRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    to_device::ack_to_device(State(state), bot_device(&p)?, Json(req)).await
}

async fn upload_cross_signing_keys(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<cross_signing::CrossSigningRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let actor = bot_device(&p)?;
    cross_signing::upload_cross_signing_keys_as(
        state,
        actor,
        cross_signing::CrossSigningAuth::BotDevice,
        req,
    )
    .await
}

/// 自己署名鍵による自分の端末への署名だけを受け付ける。他のユーザーのマスター鍵への署名（検証済みの印）は
/// 直前の再認証が要るので、Bot には出せない（Bot は TOFU だけで相手を信頼する）。
async fn upload_signatures(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<cross_signing::SignaturesRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let actor = bot_device(&p)?;
    if req
        .signatures
        .iter()
        .any(|signature| signature.target_user_id != actor.id)
    {
        return Err(AppError::Forbidden(
            "An app can sign only its own devices".into(),
        ));
    }
    cross_signing::upload_signatures(State(state), actor, HeaderMap::new(), Json(req)).await
}

/// TOFU のピンを置く。`replace: true`（置き換え）は直前の再認証が要るので、Bot には出せない。
async fn upload_pins(
    State(state): State<AppState>,
    p: Principal,
    Json(req): Json<cross_signing::PinsRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let actor = bot_device(&p)?;
    if req.replace {
        return Err(AppError::Forbidden(
            "An app cannot replace a TOFU pin".into(),
        ));
    }
    cross_signing::upload_pins(State(state), actor, HeaderMap::new(), Json(req)).await
}

async fn list_pins(
    State(state): State<AppState>,
    p: Principal,
    Query(q): Query<cross_signing::PinsQuery>,
) -> Result<Json<Value>, AppError> {
    cross_signing::list_pins(State(state), bot_device(&p)?, Query(q)).await
}

/// 配布先のスナップショット。メンバーが許可して参加した暗号化チャンネルだけ（`Principal::e2ee_channel`）。
async fn get_recipients(
    State(state): State<AppState>,
    p: Principal,
    Path(channel_id): Path<String>,
    Query(q): Query<e2ee_channels::RecipientsQuery>,
) -> Result<Json<Value>, AppError> {
    let actor = bot_device(&p)?;
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        p.e2ee_channel(&mut tx, &channel_id, false).await?;
        tx.commit().await?;
    }
    e2ee_channels::get_recipients(State(state), actor, Path(channel_id), Query(q)).await
}
