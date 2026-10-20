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
  /** 対象がワークスペース管理者か（`GET /api/users/admins` から解決する）。 */
  targetIsAdmin: boolean;
}

/**
 * メンバーキック。自分自身・チャンネル作成者・ワークスペース管理者は UI でも除外する。
 *
 * ロールは Shape に含まれないため、管理者かどうかは軽量エンドポイント
 * `GET /api/users/admins` の結果を `targetIsAdmin` として渡す。取得に失敗して
 * 判定できない場合も、バックエンドが 403 を返したらダイアログを閉じて通知する
 * （権限判定の最終防衛線はバックエンド）。
 */
export function canKickChannelMember({
  actorId,
  actorCanManage,
  channelCreatorId,
  targetUserId,
  targetIsAdmin,
}: KickPermissionInput): boolean {
  if (!actorId || !actorCanManage) {
    return false;
  }
  if (targetUserId === actorId || targetUserId === channelCreatorId) {
    return false;
  }
  return !targetIsAdmin;
}
