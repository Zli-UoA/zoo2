/** 未読の区切りを置くために、既読位置まで読み足すページ数の上限。 */
export const MAX_UNREAD_PAGES = 20;

/**
 * 未読区切りに使うメッセージ id を決める。
 *
 * 読み取り専用画面では渡された境界を優先する。既読位置の取得中・失敗時は
 * 区切りを出さず、保存済みのメッセージだけを表示する。
 */
export function resolveUnreadBoundaryId(
  unreadBoundaryId: string | null | undefined,
  firstUnreadId: string | null,
  readStateLoading: boolean,
  readStateError: boolean,
): string | null {
  return unreadBoundaryId !== undefined
    ? unreadBoundaryId
    : readStateLoading || readStateError
      ? null
      : firstUnreadId;
}

/** 未読の先頭が、いま読み込んでいる範囲に含まれるか。 */
export function isUnreadCovered(params: {
  lastRead: number | null;
  oldestCreatedAt: number;
  hasOlder: boolean;
  loadedPages: number;
}): boolean {
  const { lastRead, oldestCreatedAt, hasOlder, loadedPages } = params;
  return (
    lastRead === null ||
    lastRead <= 0 ||
    !Number.isFinite(oldestCreatedAt) ||
    oldestCreatedAt <= lastRead ||
    !hasOlder ||
    loadedPages >= MAX_UNREAD_PAGES
  );
}

export type UnreadLoadDecision = "load" | "too-old" | "none";

/** 未読の区切りを置くために、古いページをさらに読む必要があるか。 */
export function decideUnreadLoad(params: {
  readBoundary: number | null;
  oldestCreatedAt: number;
  hasOlder: boolean;
  isLoadingOlder: boolean;
  canLoadOlder: boolean;
  loadedPages: number;
}): UnreadLoadDecision {
  const { readBoundary, oldestCreatedAt, hasOlder, isLoadingOlder, canLoadOlder, loadedPages } =
    params;
  if (readBoundary === null || readBoundary <= 0) return "none";
  if (!Number.isFinite(oldestCreatedAt) || oldestCreatedAt <= readBoundary) return "none";
  if (!hasOlder || isLoadingOlder || !canLoadOlder) return "none";
  if (loadedPages >= MAX_UNREAD_PAGES) return "too-old";
  return "load";
}
