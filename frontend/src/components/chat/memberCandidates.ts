import { DEFAULT_LOCALE, INTL_LOCALES, type Locale } from "../../lib/i18n/locale";
import type { ChannelUserRow, UserRow } from "../../lib/electric/types";

/** 候補リストに一度に出す最大件数。 */
export const MEMBER_CANDIDATE_LIMIT = 8;

/** メンバー表示に使うラベル。表示名 → ユーザー名の順にフォールバックする。 */
export function userLabel(user: Pick<UserRow, "display_name" | "user_name">): string {
  return user.display_name || user.user_name;
}

/**
 * 選択言語のラベル順に並べた新しい配列を返す。
 *
 * キー入力のたびに全ユーザーを並べ替えないよう、`MembersPanel` 側でユーザー一覧に
 * 対して一度だけ適用し、`filterMemberCandidates` は並びを保ったまま絞り込む。
 */
export function sortUsersByLabel(users: UserRow[], locale: Locale = DEFAULT_LOCALE): UserRow[] {
  const collator = new Intl.Collator(INTL_LOCALES[locale]);
  return [...users].sort((a, b) => collator.compare(userLabel(a), userLabel(b)));
}

export interface MemberCandidateFilter {
  /** `sortUsersByLabel` などで表示順を整えたユーザー一覧。 */
  users: UserRow[];
  members: ChannelUserRow[];
  query: string;
  limit?: number;
}

/**
 * 追加できるメンバー候補を絞り込む。
 *
 * - 既にメンバーのユーザーは出さない（重複追加はサーバが変更を返さず `awaitTxId`
 *   がタイムアウトするため、`MembersPanel` 側のガードと二重で防ぐ）。
 * - 表示名 / ユーザー名の部分一致（大文字小文字を無視）で絞る。
 * - 並べ替えは行わず、渡された順のまま先頭 `limit` 件を返す。
 */
export function filterMemberCandidates({
  users,
  members,
  query,
  limit = MEMBER_CANDIDATE_LIMIT,
}: MemberCandidateFilter): UserRow[] {
  const memberIds = new Set(members.map((member) => member.user_id));
  const needle = query.trim().toLowerCase();
  return users
    .filter((user) => !user.is_bot && !memberIds.has(user.id))
    .filter((user) => {
      if (!needle) {
        return true;
      }
      return (
        userLabel(user).toLowerCase().includes(needle) ||
        user.user_name.toLowerCase().includes(needle)
      );
    })
    .slice(0, limit);
}

/** 作成者を先頭に固定し、残りは表示名のABC順。同名はユーザー名・IDで安定化する。 */
export function sortChannelMembers(
  members: ChannelUserRow[],
  users: UserRow[],
  creatorId: string,
): ChannelUserRow[] {
  const byId = new Map(users.map((user) => [user.id, user]));
  const collator = new Intl.Collator("en", { sensitivity: "base", numeric: true });
  return [...members].sort((a, b) => {
    const creatorOrder = Number(b.user_id === creatorId) - Number(a.user_id === creatorId);
    if (creatorOrder) return creatorOrder;
    const left = byId.get(a.user_id);
    const right = byId.get(b.user_id);
    return (
      collator.compare(left ? userLabel(left) : a.user_id, right ? userLabel(right) : b.user_id) ||
      collator.compare(left?.user_name ?? a.user_id, right?.user_name ?? b.user_id) ||
      a.user_id.localeCompare(b.user_id)
    );
  });
}
