/** スクロール位置の判断材料。リスト先頭・末尾の id だけを見る。 */
export interface ScrollSnapshot {
  firstId: string | null;
  lastId: string | null;
}

export type ScrollChange = "initial" | "append" | "prepend" | "replace" | "none";

/** id 列からスナップショットを作る。 */
export function snapshot(ids: string[]): ScrollSnapshot {
  if (ids.length === 0) {
    return { firstId: null, lastId: null };
  }
  return { firstId: ids[0], lastId: ids[ids.length - 1] };
}

/**
 * 直前と今回のリストを比べ、変化の種類を判定する。
 *
 * - `initial`: 空 → 非空（初回ロード）。最新へ移動する。
 * - `append`: 末尾に追加（新規受信）。atBottom のときだけ追従する。
 * - `prepend`: 先頭に追加（古いページ）。スクロール位置を補正して維持する。
 * - `replace`: 先頭も末尾も変わった（チャンネル切替など）。最新へ移動する。
 * - `none`: 変化なし、または中央の更新のみ。
 */
export function classifyScrollChange(previous: ScrollSnapshot, next: ScrollSnapshot): ScrollChange {
  if (previous.firstId === null && previous.lastId === null) {
    return next.firstId === null && next.lastId === null ? "none" : "initial";
  }
  if (next.firstId === null && next.lastId === null) {
    return "none";
  }
  if (next.firstId === previous.firstId && next.lastId === previous.lastId) {
    return "none";
  }
  if (next.firstId === previous.firstId) {
    return "append";
  }
  if (next.lastId === previous.lastId) {
    return "prepend";
  }
  return "replace";
}

/** その変化のあと、最新（末尾）へ追従すべきか。 */
export function shouldStickToBottom(change: ScrollChange, atBottom: boolean): boolean {
  if (change === "initial" || change === "replace") {
    return true;
  }
  if (change === "append") {
    return atBottom;
  }
  return false;
}

/** ビューポート下端から `threshold` px 以内なら「最下部」とみなす。 */
export function isNearBottom(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
  threshold = 48,
): boolean {
  return scrollHeight - scrollTop - clientHeight <= threshold;
}
