use crate::{error::AppError, models::UserRole};

/// ユーザーがワークスペース管理者 (admin ロール) かどうかを判定
pub async fn is_workspace_admin(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    tenant_id: &str,
    dev_auth_enabled: bool,
) -> Result<bool, AppError> {
    // 開発環境かつ固定開発ユーザーの場合は常に管理者として扱う (テスト・ローカル環境の便宜)
    if dev_auth_enabled
        && tenant_id == crate::auth::tenant::DEFAULT_TENANT_ID
        && user_id == crate::auth::DEFAULT_DEV_USER_ID
    {
        return Ok(true);
    }

    let user_roles = UserRole::filter(
        UserRole::fields()
            .user_id()
            .eq(user_id)
            .and(UserRole::fields().tenant_id().eq(tenant_id)),
    )
    .exec(tx)
    .await?;

    for ur in user_roles {
        if ur.role_id == "role_admin" {
            return Ok(true);
        }
    }

    Ok(false)
}

/// チャンネル管理権限 (更新・アーカイブ・削除)
/// - チャンネル作成者 OR ワークスペース管理者
pub fn can_manage_channel(channel_creator_id: &str, actor_id: &str, is_admin: bool) -> bool {
    channel_creator_id == actor_id || is_admin
}

/// メッセージ削除権限
/// - 投稿者本人 OR チャンネル作成者 OR ワークスペース管理者
pub fn can_delete_message(
    message_author_id: &str,
    channel_creator_id: &str,
    actor_id: &str,
    is_admin: bool,
) -> bool {
    message_author_id == actor_id || channel_creator_id == actor_id || is_admin
}

/// メンバー追加権限
/// - ワークスペース管理者: 常に追加可能
/// - チャンネル作成者: 常に追加可能
/// - プライベートチャンネル: 作成者または管理者のみ追加可能
/// - パブリックチャンネル: 既存メンバーも追加可能
pub fn can_add_member(
    is_private: bool,
    channel_creator_id: &str,
    actor_id: &str,
    is_actor_member: bool,
    is_admin: bool,
) -> bool {
    if is_admin || channel_creator_id == actor_id {
        return true;
    }
    if is_private { false } else { is_actor_member }
}

/// メンバーキック権限 (チャンネルからユーザーを除外)
/// - 自身を指定した場合はキック不可 (leave を使うべき)
/// - チャンネル作成者をキックすることは不可
/// - ワークスペース管理者をキックすることは不可 (管理者同士のキックも保護)
/// - 実行者: チャンネル作成者 OR ワークスペース管理者
pub fn can_kick_member(
    channel_creator_id: &str,
    actor_id: &str,
    target_user_id: &str,
    actor_is_admin: bool,
    target_is_admin: bool,
) -> bool {
    // 自身はキック不可
    if actor_id == target_user_id {
        return false;
    }
    // チャンネル作成者はキック不可
    if target_user_id == channel_creator_id {
        return false;
    }
    // ワークスペース管理者はキック不可 (管理者同士のキックも保護)
    if target_is_admin {
        return false;
    }
    // 実行者はチャンネル作成者またはワークスペース管理者であること
    channel_creator_id == actor_id || actor_is_admin
}

/// チャンネル閲覧可能か
pub fn is_channel_visible(is_private: bool, is_member: bool) -> bool {
    !is_private || is_member
}

/// チャンネル直接参加 (join) 可能か
pub fn can_join(is_private: bool) -> bool {
    !is_private
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_manage_channel() {
        assert!(can_manage_channel("alice", "alice", false));
        assert!(can_manage_channel("alice", "bob", true));
        assert!(!can_manage_channel("alice", "bob", false));
    }

    #[test]
    fn test_can_delete_message() {
        // 投稿者本人
        assert!(can_delete_message("alice", "charlie", "alice", false));
        // チャンネル作成者
        assert!(can_delete_message("alice", "charlie", "charlie", false));
        // ワークスペース管理者
        assert!(can_delete_message("alice", "charlie", "bob", true));
        // 無関係な他者
        assert!(!can_delete_message("alice", "charlie", "bob", false));
    }

    #[test]
    fn test_can_add_member() {
        // 管理者
        assert!(can_add_member(true, "alice", "admin", false, true));
        assert!(can_add_member(false, "alice", "admin", false, true));
        // チャンネル作成者
        assert!(can_add_member(true, "alice", "alice", false, false));
        assert!(can_add_member(false, "alice", "alice", false, false));
        // プライベートチャンネルでの一般メンバー (追加不可)
        assert!(!can_add_member(true, "alice", "bob", true, false));
        // パブリックチャンネルでの一般メンバー (追加可能)
        assert!(can_add_member(false, "alice", "bob", true, false));
        // パブリックチャンネルでの非メンバー (追加不可)
        assert!(!can_add_member(false, "alice", "bob", false, false));
    }

    #[test]
    fn test_can_kick_member() {
        let creator = "alice";
        let admin = "admin_user";
        let normal_user = "bob";

        // 作成者が一般メンバーをキック -> 可
        assert!(can_kick_member(creator, creator, normal_user, false, false));
        // 管理者が一般メンバーをキック -> 可
        assert!(can_kick_member(creator, admin, normal_user, true, false));
        // 一般メンバーが他者をキック -> 不可
        assert!(!can_kick_member(
            creator,
            normal_user,
            "charlie",
            false,
            false
        ));
        // 自身を指定 -> 不可
        assert!(!can_kick_member(creator, creator, creator, false, false));
        assert!(!can_kick_member(creator, admin, admin, true, true));
        // チャンネル作成者をキック -> 管理者であっても不可
        assert!(!can_kick_member(creator, admin, creator, true, false));
        // チャンネル作成者がワークスペース管理者をキック -> 不可
        assert!(!can_kick_member(creator, creator, admin, false, true));
        // ワークスペース管理者が別のワークスペース管理者をキック -> 不可 (管理者キック保護)
        assert!(!can_kick_member(creator, admin, "other_admin", true, true));
    }
}
