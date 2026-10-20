/**
 * チャンネル操作の UI 権限。条件はバックエンドの `auth::rbac` に合わせる。
 *
 * `isAdmin` は `GET /api/users/me` の `is_admin` から取る。チャンネル作成者か
 * ワークスペース管理者だけがチャンネル管理（アーカイブ・キック・強制削除）を
 * 行える。
 */

export interface ManagePermissionInput {
  isAdmin: boolean;
  isCreator: boolean;
}

/** チャンネルの管理（更新・アーカイブ・キック・強制削除）。 */
export function canManageChannel({ isAdmin, isCreator }: ManagePermissionInput): boolean {
  return isAdmin || isCreator;
}

export interface AddMemberPermissionInput extends ManagePermissionInput {
  isPrivate: boolean;
  isMember: boolean;
}

/** メンバー追加。管理者・作成者は常に可、公開は既存メンバーも可。 */
export function canAddChannelMember({
  isAdmin,
  isCreator,
  isPrivate,
  isMember,
}: AddMemberPermissionInput): boolean {
  if (isAdmin || isCreator) {
    return true;
  }
  return !isPrivate && isMember;
}

export interface KickPermissionInput {
  actorId: string | null;
  actorCanManage: boolean;
  channelCreatorId: string;
  targetUserId: string;
}

/**
 * メンバーキック。自分自身とチャンネル作成者は UI でも除外する。
 *
 * ワークスペース管理者かどうかは Shape にロールが無いため対象ユーザーについて
 * 判定できない。対象が管理者の場合はバックエンドが 403 を返すので、失敗を
 * `sonner` で通知する（権限判定の最終防衛線はバックエンド）。
 */
export function canKickChannelMember({
  actorId,
  actorCanManage,
  channelCreatorId,
  targetUserId,
}: KickPermissionInput): boolean {
  if (!actorId || !actorCanManage) {
    return false;
  }
  if (targetUserId === actorId || targetUserId === channelCreatorId) {
    return false;
  }
  return true;
}
