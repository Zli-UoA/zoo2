import type { VirtualItem } from "@tanstack/react-virtual";

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
 * - `replace`: 先頭も末尾も変わり、直前の末尾も残っていない（チャンネル切替など）。最新へ移動する。
 * - `none`: 変化なし、中央の更新、または末尾が消えただけ（論理削除）。
 *
 * `nextIds` は今回のリストの id 集合。末尾だけが変わったときに、その変化が
 * 「追加」なのか「末尾の削除」なのかを、直前の末尾 id がまだ残っているかで見分ける。
 */
export function classifyScrollChange(
  previous: ScrollSnapshot,
  next: ScrollSnapshot,
  nextIds: ReadonlySet<string>,
): ScrollChange {
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
    // 直前の末尾がまだ残っているときだけ「新規追加」とみなす。
    // 末尾が論理削除などで消えただけの場合は append にしない。
    return previous.lastId !== null && nextIds.has(previous.lastId) ? "append" : "none";
  }
  if (next.lastId === previous.lastId) {
    return "prepend";
  }
  // 件数の上限で、新着の分だけ最古の行が外れた (先頭も末尾も変わったが、直前の末尾は残っている)。
  // 別のチャンネルへの切替 (replace) とは違い、新規追加として扱う。
  if (previous.lastId !== null && nextIds.has(previous.lastId)) {
    return "append";
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

/**
 * 視差効果を減らす設定が有効なときは smooth スクロールを即時に置き換える。
 * CSS のグローバル設定では JS の `scrollTo` / `scrollToIndex` の挙動は変わらないため、
 * スクロールだけは呼び出し側で明示的に切り替える。
 */
export function resolveScrollBehavior(
  behavior: ScrollBehavior,
  prefersReducedMotion: boolean,
): ScrollBehavior {
  return prefersReducedMotion ? "auto" : behavior;
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

const MAX_CHANNEL_SCROLL_CACHE = 32;
const MAX_CACHED_ROW_SIZES = 200;
const MIN_ESTIMATED_ROW_HEIGHT = 40;
const MAX_ESTIMATED_ROW_HEIGHT = 400;

export interface ChannelMeasurementEntry {
  containerWidth: number;
  viewportHeight: number;
  estimatedRowHeight: number;
  sizes: ReadonlyMap<string, number>;
}

export interface InitialScrollPlan {
  initialMeasurementsCache: VirtualItem[] | undefined;
  initialOffset: number;
  initialRect: { width: number; height: number };
  estimatedRowHeight: number;
  cachedWidth: number | undefined;
  cachedSizes: ReadonlyMap<string, number> | undefined;
}

const channelMeasurementCache = new Map<string, ChannelMeasurementEntry>();
let cacheGeneration = 0;
let lastViewportRect: { width: number; height: number } | undefined;

/**
 * ユーザーとチャンネルからタブ内計測キャッシュのキーを組み立てる。
 *
 * 未ログイン／読み取り専用などで `currentUserId` が `null` の場合は空文字プレフィックス
 * (`"\0<channelId>"`) を意図的に共有し、ログアウト時は `clearChannelMeasurementCache()` で一括破棄する。
 */
export function buildChannelScrollKey(channelId: string, currentUserId: string | null): string {
  return `${currentUserId ?? ""}\0${channelId}`;
}

/** チャンネル別計測キャッシュの世代番号を返す（クリア後の古いアンマウント書戻し抑止用）。 */
export function getChannelMeasurementCacheGeneration(): number {
  return cacheGeneration;
}

/** チャンネル別の実測高さキャッシュを取得する（LRU 順を更新）。 */
export function getChannelMeasurementCache(key: string): ChannelMeasurementEntry | undefined {
  const entry = channelMeasurementCache.get(key);
  if (!entry) return undefined;
  channelMeasurementCache.delete(key);
  channelMeasurementCache.set(key, entry);
  return entry;
}

/** テストやサインアウト向けにキャッシュを初期化する。 */
export function clearChannelMeasurementCache(): void {
  cacheGeneration += 1;
  channelMeasurementCache.clear();
  lastViewportRect = undefined;
}

/** 直近に観測したチャット一覧のビューポート寸法を記録する（未訪問チャンネルの初期描画にも利用）。 */
export function recordLastViewportRect(width: number, height: number): void {
  if (width > 0 && height > 0) {
    lastViewportRect = { width, height };
  }
}

/**
 * 仮想リストの現在のスナップショットとコンテナ寸法から、
 * 次回切替時の再推定ズレを防ぐための実測高さキャッシュを更新する。
 */
export function saveChannelMeasurementCache(params: {
  key: string;
  containerWidth: number;
  viewportHeight: number;
  snapshotItems: readonly VirtualItem[];
  previousSizes?: ReadonlyMap<string, number>;
  fallbackRowHeight: number;
}): void {
  const { key, containerWidth, viewportHeight, snapshotItems, previousSizes, fallbackRowHeight } =
    params;
  if (containerWidth <= 0 || viewportHeight <= 0 || snapshotItems.length === 0) {
    return;
  }

  const prev = channelMeasurementCache.get(key);
  const widthChanged = prev && Math.abs(prev.containerWidth - containerWidth) > 2;
  const merged = new Map<string, number>(widthChanged ? undefined : (prev?.sizes ?? previousSizes));

  for (const item of snapshotItems) {
    if (typeof item.key === "string" && Number.isFinite(item.size) && item.size > 0) {
      merged.delete(item.key);
      merged.set(item.key, item.size);
    }
  }

  if (merged.size === 0) return;

  if (merged.size > MAX_CACHED_ROW_SIZES) {
    const excess = merged.size - MAX_CACHED_ROW_SIZES;
    let removed = 0;
    for (const k of merged.keys()) {
      if (removed >= excess) break;
      merged.delete(k);
      removed += 1;
    }
  }

  let total = 0;
  for (const size of merged.values()) total += size;
  const mean = Math.round(total / merged.size);
  const estimatedRowHeight = Math.min(
    MAX_ESTIMATED_ROW_HEIGHT,
    Math.max(MIN_ESTIMATED_ROW_HEIGHT, mean || fallbackRowHeight),
  );

  channelMeasurementCache.delete(key);
  channelMeasurementCache.set(key, {
    containerWidth,
    viewportHeight,
    estimatedRowHeight,
    sizes: merged,
  });
  lastViewportRect = { width: containerWidth, height: viewportHeight };

  if (channelMeasurementCache.size > MAX_CHANNEL_SCROLL_CACHE) {
    const oldestKey = channelMeasurementCache.keys().next().value;
    if (oldestKey !== undefined) {
      channelMeasurementCache.delete(oldestKey);
    }
  }
}

/**
 * 初回描画時の仮想スクロールキャッシュと開始オフセットを計算する。
 * キャッシュが無い場合でも末尾付近から描画を開始し、先頭行の不要なマウントと
 * 上→下のスクロール跳ねを防ぐ。
 */
export function computeInitialScrollPlan(params: {
  itemIds: readonly string[];
  cached?: ChannelMeasurementEntry;
  defaultRowHeight: number;
  defaultViewportHeight: number;
  targetIndex: number;
  unreadIndex: number;
}): InitialScrollPlan {
  const { itemIds, cached, defaultRowHeight, defaultViewportHeight, targetIndex, unreadIndex } =
    params;
  const estimatedRowHeight = cached?.estimatedRowHeight ?? defaultRowHeight;
  const initialRect = cached
    ? { width: cached.containerWidth, height: cached.viewportHeight }
    : (lastViewportRect ?? { width: 0, height: defaultViewportHeight });
  const viewportHeight = initialRect.height;
  const sizes = cached?.sizes;

  let offset = 0;
  const measurements: VirtualItem[] = [];
  let targetStart = -1;
  let targetSize = estimatedRowHeight;
  let unreadStart = -1;

  for (let index = 0; index < itemIds.length; index += 1) {
    const id = itemIds[index]!;
    const cachedSize = sizes?.get(id);
    const size = cachedSize ?? estimatedRowHeight;
    if (cachedSize !== undefined) {
      measurements.push({
        key: id,
        index,
        start: offset,
        end: offset + size,
        size,
        lane: 0,
      });
    }
    if (index === targetIndex) {
      targetStart = offset;
      targetSize = size;
    }
    if (index === unreadIndex) {
      unreadStart = offset;
    }
    offset += size;
  }

  let initialOffset = Math.max(0, offset - viewportHeight);
  if (targetIndex >= 0) {
    initialOffset = Math.max(0, targetStart - Math.max(0, (viewportHeight - targetSize) / 2));
  } else if (unreadIndex >= 0) {
    initialOffset = Math.max(0, unreadStart);
  }

  return {
    initialMeasurementsCache: measurements.length > 0 ? measurements : undefined,
    initialOffset,
    initialRect,
    estimatedRowHeight,
    cachedWidth: cached?.containerWidth,
    cachedSizes: sizes,
  };
}
