//! 既存の非公開チャンネルと DM を暗号化に切り替える（#367）。仕様は `docs/e2ee.md` の「既存のチャンネルの暗号化」。
//!
//! `POST /api/channels/{id}/encrypt` は、`channels.encrypted` を false から true にだけ変える（戻せない）。
//! 切り替える前のメッセージは平文のまま残り、切り替えたあとのメッセージだけを暗号化する。
//!
//! ロックの順は `routes/e2ee.rs` の先頭のとおり：メンバー単位のロック（今のメンバー全員、`user_id` の昇順）、
//! `channels` の行（`FOR UPDATE`）。`FOR UPDATE` は、平文の経路が取る `FOR KEY SHARE`（外部キーの検査、
//! `e2ee_channels::lock_encryption`、DB のトリガー）と衝突するので、平文の送信、メンバーの変更、Slack の取り込みは、
//! 切り替えの前か後のどちらかに並ぶ。

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use uuid::Uuid;

use crate::{
    auth::{
        AuthUser,
        rbac::{can_manage_channel, is_workspace_admin},
    },
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser},
    routes::{
        channels::ChannelResponse,
        e2ee::{first, limit_exceeded, rows},
        e2ee_channels::{
            self, MAX_ENCRYPTED_CHANNEL_MEMBERS, apps_forbidden, conversion_blocked, disabled,
            public_forbidden,
        },
        response::ApiResponse,
    },
    state::AppState,
};

/// 読んだメンバーと、チャンネルの行をロックしたあとのメンバーが違ったときに、最初からやり直す回数。
const ATTEMPTS: usize = 3;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/encrypt", post(encrypt_channel))
}

pub(crate) async fn encrypt_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;
    for _ in 0..ATTEMPTS {
        if let Some(response) = try_encrypt(&state, &auth_user, &id).await? {
            return Ok(Json(response));
        }
    }
    Err(AppError::Conflict(
        "The channel members changed while encrypting the channel; retry".into(),
    ))
}

/// 1 回の試み。メンバーの読み直しが合わなければ `None`（ロールバックしてやり直す）。
async fn try_encrypt(
    state: &AppState,
    auth_user: &AuthUser,
    id: &str,
) -> Result<Option<ApiResponse<ChannelResponse>>, AppError> {
    let not_found = || AppError::NotFound(format!("Channel not found with id: {id}"));
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;

    let channel = Channel::get_by_id(&mut tx, id).await.map_err(|err| {
        if err.is_record_not_found() {
            not_found()
        } else {
            AppError::from(err)
        }
    })?;
    if channel.deleted_at.is_some() {
        return Err(not_found());
    }
    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id)
            .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .is_some();
    // 非公開チャンネルと DM は、メンバーでなければ存在を隠す（管理者でも、メンバーでなければ切り替えられない。
    // 暗号化チャンネルでメンバーでない管理者が自分を足せないのと同じ。docs/e2ee.md の E2EE_ADD_FORBIDDEN）。
    if (channel.is_private || channel.is_dm) && !member {
        return Err(not_found());
    }
    // App（Bot）のトークンは、作成と同じく、設定より先に拒否する。
    let bot = crate::apps::one(
        &mut tx,
        "SELECT json_build_object('bot', EXISTS (SELECT 1 FROM users WHERE id = $1 AND is_bot))::text",
        &[auth_user.id.as_str()],
    )
    .await?["bot"]
        == true;
    if bot {
        return Err(apps_forbidden());
    }
    if !channel.is_private {
        return Err(public_forbidden());
    }
    // すでに暗号化されていれば、何もせずに返す（応答をなくした再送）。
    if channel.encrypted {
        return Ok(Some(ApiResponse::new(ChannelResponse::from(channel), None)));
    }
    if !state.config.e2ee_encrypted_channels_enabled {
        return Err(disabled());
    }
    // 非公開チャンネルは、チャンネルを管理できる人（作成者か管理者）だけ。DM とグループ DM は、メンバーなら誰でも。
    if !channel.is_dm {
        let is_admin =
            is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?;
        if !can_manage_channel(&channel.created_user_id, &auth_user.id, is_admin, false) {
            return Err(AppError::Forbidden(
                "Only the channel creator or a workspace admin can encrypt this channel".into(),
            ));
        }
    }
    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot encrypt an archived channel".into(),
        ));
    }

    // メンバー単位のロック（端末の削除などでの一括の epoch の増加と直列にする）を、チャンネルの行より前に取る。
    // 一括の増加が、切り替えの前の参加先（平文なので対象外）を確定してから、切り替えのあとにコミットすると、
    // 削除した端末が切り替えのあとのスナップショットに残るため。
    let members = member_ids(&mut tx, id).await?;
    if members.len() as i64 > MAX_ENCRYPTED_CHANNEL_MEMBERS {
        return Err(limit_exceeded("members"));
    }
    let member_refs: Vec<&str> = members.iter().map(String::as_str).collect();
    e2ee_channels::lock_members(&mut tx, &member_refs).await?;

    let locked = first(
        &mut tx,
        "SELECT json_build_object('encrypted', encrypted, 'deleted', deleted_at IS NOT NULL,
           'archived', archive_at IS NOT NULL, 'is_private', is_private, 'channel_name', channel_name)::text
         FROM channels WHERE id = $1 FOR UPDATE",
        &[id],
    )
    .await?
    .ok_or_else(not_found)?;
    if locked["deleted"] == true {
        return Err(not_found());
    }
    if locked["encrypted"] == true {
        let channel = Channel::get_by_id(&mut tx, id).await?;
        tx.commit().await?;
        return Ok(Some(ApiResponse::new(ChannelResponse::from(channel), None)));
    }
    if locked["is_private"] != true {
        return Err(public_forbidden());
    }
    if locked["archived"] == true {
        return Err(AppError::BadRequest(
            "Cannot encrypt an archived channel".into(),
        ));
    }
    // ロックより前に読んだメンバーと違えば（そのあいだに平文の経路で参加、退出した人がいる）、やり直す。
    // 足された人はメンバー単位のロックを取っていないので、そのまま切り替えると、上の直列化が崩れる。
    if member_ids(&mut tx, id).await? != members {
        let _ = tx.rollback().await;
        return Ok(None);
    }
    if !members.iter().any(|m| m == &auth_user.id) {
        return Err(not_found());
    }

    let channel_name = locked["channel_name"].as_str().unwrap_or_default();
    let renamed = encrypted_dm_name(channel_name);
    let mut blockers = Vec::new();
    let found = first(
        &mut tx,
        "SELECT json_build_object(
           'apps', EXISTS (SELECT 1 FROM channel_user cu JOIN users u ON u.id = cu.user_id
             WHERE cu.channel_id = $1 AND u.is_bot
               AND NOT EXISTS (SELECT 1 FROM e2ee_bot_grants g
                 WHERE g.channel_id = cu.channel_id AND g.bot_user_id = cu.user_id)),
           'webhooks', EXISTS (SELECT 1 FROM channel_webhooks WHERE channel_id = $1),
           'encrypted_dm', $2 <> '' AND EXISTS (SELECT 1 FROM channels WHERE channel_name = $2))::text",
        &[id, renamed.as_deref().unwrap_or("")],
    )
    .await?
    .unwrap_or_default();
    for blocker in ["apps", "webhooks", "encrypted_dm"] {
        if found[blocker] == true {
            blockers.push(blocker);
        }
    }
    if !blockers.is_empty() {
        return Err(conversion_blocked(&blockers));
    }

    // 切り替えでも epoch を増やす。切り替えの前に Shape で見ていた epoch のまま、送信の準備を進めないため。
    let now = crate::models::now_utc_millis();
    toasty::sql::statement(
        "UPDATE channels SET encrypted = true, membership_epoch = membership_epoch + 1,
           channel_name = COALESCE(NULLIF($2, ''), channel_name), updated_at = $3::timestamp
         WHERE id = $1",
    )
    .bind(id)
    .bind(renamed.as_deref().unwrap_or(""))
    .bind(now.to_string())
    .exec(&mut tx)
    .await?;

    let channel = Channel::get_by_id(&mut tx, id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Some(ApiResponse::new(ChannelResponse::from(channel), txid)))
}

/// チャンネルのメンバーの `user_id`（昇順）。
async fn member_ids(tx: &mut toasty::Transaction<'_>, id: &str) -> Result<Vec<String>, AppError> {
    Ok(rows(
        tx,
        "SELECT json_build_object('user_id', user_id)::text FROM channel_user
         WHERE channel_id = $1 ORDER BY user_id COLLATE \"C\"",
        &[id],
    )
    .await?
    .into_iter()
    .filter_map(|row| row["user_id"].as_str().map(str::to_owned))
    .collect())
}

/// 1 対 1 の平文の DM（`dm:{a}:{b}`）は、暗号化 DM の名前（`edm:{a}:{b}`）に変える。平文の DM を求める
/// `POST /api/dms` が、切り替えた DM を返さない（新しい平文の DM を作る）ようにするため。グループ DM と
/// チャンネルは名前を変えない。
pub(crate) fn encrypted_dm_name(channel_name: &str) -> Option<String> {
    let rest = channel_name.strip_prefix("dm:")?;
    if rest.starts_with("group:") {
        return None;
    }
    Some(format!("edm:{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_only_one_to_one_direct_messages() {
        assert_eq!(encrypted_dm_name("dm:a:b").as_deref(), Some("edm:a:b"));
        assert_eq!(encrypted_dm_name("dm:group:x"), None);
        assert_eq!(encrypted_dm_name("edm:a:b"), None);
        assert_eq!(encrypted_dm_name("general"), None);
    }
}
