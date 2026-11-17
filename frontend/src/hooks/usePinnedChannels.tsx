import { useLiveQuery } from "@tanstack/react-db";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError, createConfiguredApiClient } from "../lib/api/client";
import type { ChannelPinsResponse, UserPinnedChannelResponse } from "../lib/api/types";
import { useCollections } from "../lib/electric/collections-context";
import { createChannelPinWrites } from "../lib/electric/writes";
import type { UserPinnedChannelRow } from "../lib/electric/types";
import { localizeApiError } from "../lib/i18n/apiError";
import { useCurrentUser } from "./useCurrentUser";

/** 並び替えの書き込みを表す保留キー。チャンネル ID と衝突しない値。 */
const REORDER_KEY = "__order__";
const CONFLICT_CODE = "CHANNEL_PINS_CONFLICT";
const MAX_ATTEMPTS = 3;

type AnnouncementKey =
  | "channelPins.announce.pinned"
  | "channelPins.announce.unpinned"
  | "channelPins.announce.moved"
  | "channelPins.announce.reorderConflict"
  | "channelPins.announce.reorderArchived";

export interface PinnedChannelsValue {
  /** Provider とコレクションがそろい、ピン留めを操作できるか。 */
  available: boolean;
  /** 位置順のピン。アーカイブ中のチャンネルも含む（表示側で絞る）。 */
  pins: UserPinnedChannelResponse[];
  version: number;
  loading: boolean;
  /** 書き込み中のチャンネル ID（並び替え中は `__order__`）。 */
  pending: ReadonlySet<string>;
  error: string | null;
  /** スクリーンリーダー向けの操作結果。 */
  announcement: string;
  isPinned: (channelId: string) => boolean;
  pinChannel: (channelId: string) => Promise<void>;
  unpinChannel: (channelId: string) => Promise<void>;
  /**
   * 表示中の並びを受けて並び替える。`nextOrder` は移動後の有効なピンの並び、
   * `movedChannelId` は動かしたチャンネル。競合したときは最新の状態の上で
   * 同じ移動をやり直し、相手が先に外していたら復活させない。
   */
  reorderPins: (nextOrder: string[], movedChannelId: string) => Promise<void>;
  refresh: () => Promise<void>;
  dismissError: () => void;
}

const EMPTY_VALUE: PinnedChannelsValue = {
  available: false,
  pins: [],
  version: 0,
  loading: false,
  pending: new Set(),
  error: null,
  announcement: "",
  isPinned: () => false,
  pinChannel: async () => {},
  unpinChannel: async () => {},
  reorderPins: async () => {},
  refresh: async () => {},
  dismissError: () => {},
};

const PinnedChannelsContext = createContext<PinnedChannelsValue | null>(null);

/** ピン留めしたチャンネルを、全画面で共有する。 */
export function PinnedChannelsProvider({ children }: { children: ReactNode }) {
  const { value, announcementId } = usePinnedChannelsState();
  return (
    <PinnedChannelsContext.Provider value={value}>
      {/* 操作の結果は常時マウントした領域で読み上げる。表示中の節が隠れていても、
          同じ操作を続けても、`key` の更新で必ず読み上げ直す。 */}
      <div role="status" aria-live="polite" className="sr-only">
        <span key={announcementId}>{value.announcement}</span>
      </div>
      {children}
    </PinnedChannelsContext.Provider>
  );
}

/**
 * ピン留めの状態。
 *
 * Provider の外（部品単体のテストなど）では、操作できない空の状態を返す。
 */
export function usePinnedChannels(): PinnedChannelsValue {
  return useContext(PinnedChannelsContext) ?? EMPTY_VALUE;
}

/** 位置と channel_id で安定した順に並べる。 */
export function sortPins(
  pins: readonly (UserPinnedChannelResponse | UserPinnedChannelRow)[],
): UserPinnedChannelResponse[] {
  return [...pins].sort(
    (a, b) => a.position - b.position || a.channel_id.localeCompare(b.channel_id),
  );
}

/** Electric の行と API の行を、位置と ID だけで比べるための署名。 */
export function pinsSignature(pins: readonly { channel_id: string; position: number }[]): string {
  return [...pins]
    .sort((a, b) => a.position - b.position || a.channel_id.localeCompare(b.channel_id))
    .map((pin) => `${pin.channel_id}:${pin.position}`)
    .join("|");
}

/** 409 `CHANNEL_PINS_CONFLICT` の本文から、最新の版とピンを取り出す。 */
export function channelPinsConflict(cause: unknown): ChannelPinsResponse | null {
  if (!(cause instanceof ApiError) || cause.status !== 409) {
    return null;
  }
  const body = cause.body;
  if (!body || typeof body !== "object") {
    return null;
  }
  const record = body as {
    code?: unknown;
    version?: unknown;
    pins?: unknown;
  };
  if (record.code !== CONFLICT_CODE || typeof record.version !== "number") {
    return null;
  }
  if (!Array.isArray(record.pins)) {
    return null;
  }
  return { version: record.version, pins: record.pins as UserPinnedChannelResponse[] };
}

/** ピン留めの上限に当たったか。 */
export function isChannelPinsLimit(cause: unknown): boolean {
  return (
    cause instanceof ApiError &&
    cause.status === 400 &&
    typeof cause.body === "object" &&
    cause.body !== null &&
    (cause.body as { code?: unknown }).code === "CHANNEL_PINS_LIMIT_EXCEEDED"
  );
}

/**
 * 待機中の書き込みが、開始時と別の世代・別のユーザーになっていないか。
 *
 * ユーザー切り替えの effect は描画後に走るため、世代だけでは切り替え直後の
 * 待機タスクを止められない。ユーザー ID も比較して、別アカウントとして送らない。
 */
export function isStalePinWrite(
  requestGeneration: number,
  currentGeneration: number,
  requestUserId: string,
  currentUserId: string | null,
): boolean {
  return requestGeneration !== currentGeneration || requestUserId !== currentUserId;
}

/** `base` から `moved` を取り出し、`targetIndex` に入れた並び。 */
export function moveWithin(base: readonly string[], moved: string, targetIndex: number): string[] {
  const rest = base.filter((id) => id !== moved);
  const index = Math.max(0, Math.min(targetIndex, rest.length));
  rest.splice(index, 0, moved);
  return rest;
}

/**
 * 並び替えの応答を待たずに表示へ反映する。アーカイブ中のピンは元のスロットに
 * 残し、有効なピンだけを `nextOrder` の順に当てはめる（サーバーの
 * `build_pin_order` と同じ考え方）。
 */
export function reorderPinsLocally(
  pins: readonly (UserPinnedChannelResponse | UserPinnedChannelRow)[],
  nextOrder: readonly string[],
): UserPinnedChannelResponse[] {
  const active = sortPins(pins);
  const byId = new Map(active.map((pin) => [pin.channel_id, pin]));
  let nextIndex = 0;
  const ordered = active.map((pin) => {
    const archived = "archived" in pin && pin.archived === true;
    if (archived) {
      return pin;
    }
    const channelId = nextOrder[nextIndex] ?? pin.channel_id;
    nextIndex += 1;
    return byId.get(channelId) ?? pin;
  });
  return ordered.map((pin, position) => ({ ...pin, position }));
}

/**
 * 競合したあとに送り直す並び。他端末が追加したピンは最新のサーバ順で残し、
 * `intendedOrder` で `movedChannelId` の直後に置いた既存要素の前（末尾へ動かした
 * ときは直前の既存要素の後ろ）へ、動かした項目だけを入れ直す。
 */
export function mergeReorder(
  intendedOrder: readonly string[],
  movedChannelId: string,
  latestActiveIds: readonly string[],
): string[] {
  const base = latestActiveIds.filter((id) => id !== movedChannelId);
  const movedIndex = intendedOrder.indexOf(movedChannelId);
  if (movedIndex < 0) {
    return base;
  }
  // 動かした項目より後ろで、最新にも残っている最初の要素の前に置く。
  for (let index = movedIndex + 1; index < intendedOrder.length; index += 1) {
    const anchor = base.indexOf(intendedOrder[index]!);
    if (anchor >= 0) {
      base.splice(anchor, 0, movedChannelId);
      return base;
    }
  }
  // 末尾へ動かした場合は、直前の既存要素の後ろに置く。
  for (let index = movedIndex - 1; index >= 0; index -= 1) {
    const anchor = base.indexOf(intendedOrder[index]!);
    if (anchor >= 0) {
      base.splice(anchor + 1, 0, movedChannelId);
      return base;
    }
  }
  base.splice(movedIndex === 0 ? 0 : base.length, 0, movedChannelId);
  return base;
}

function usePinnedChannelsState(): {
  value: PinnedChannelsValue;
  announcementId: number;
} {
  const { t } = useTranslation();
  const { currentUserId } = useCurrentUser();
  const collections = useCollections();
  const config = collections.config;
  const collectionFactory = collections.userPinnedChannels;
  const api = useMemo(
    () => createChannelPinWrites({ client: createConfiguredApiClient(config) }),
    [config],
  );
  const collection = useMemo(
    () =>
      currentUserId && typeof collectionFactory === "function"
        ? collectionFactory(currentUserId)
        : null,
    [collectionFactory, currentUserId],
  );
  const live = useLiveQuery(
    (q) => (collection ? q.from({ pin: collection }) : undefined),
    [collection],
  );

  const [pins, setPins] = useState<UserPinnedChannelResponse[]>([]);
  const pinsRef = useRef(pins);
  pinsRef.current = pins;
  const [version, setVersion] = useState(0);
  const [loading, setLoading] = useState(Boolean(currentUserId));
  const [pending, setPending] = useState<ReadonlySet<string>>(() => new Set());
  const [loadError, setLoadError] = useState<string | null>(null);
  const [writeError, setWriteError] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const [announcementId, setAnnouncementId] = useState(0);

  // アカウント切り替えの effect より先に、待機中の書き込みが古いユーザーのものか判断できるようにする。
  const currentUserIdRef = useRef(currentUserId);
  currentUserIdRef.current = currentUserId;
  const generation = useRef(0);
  const versionRef = useRef(0);
  const pendingRef = useRef<ReadonlySet<string>>(new Set());
  const serverReady = useRef(false);
  const lastLiveSignature = useRef<string | null>(null);
  // GET の開始順。先発の失敗が、後発の成功を上書きしないようにする。
  const fetchSequence = useRef(0);
  const lastSuccessfulFetch = useRef(0);
  // 権威状態を採用した回数。並び替え中の採用をロールバックで取り消さない。
  const adoptCount = useRef(0);
  // ピン留めの書き込みは 1 本ずつ順番に送る。前の書き込みの結果を採用してから
  // 次の版を読むので、同時操作で版が食い違わない。
  const writeQueue = useRef<Promise<unknown>>(Promise.resolve());

  const enqueueWrite = useCallback(<T,>(task: () => Promise<T>): Promise<T> => {
    const run = writeQueue.current.then(task, task);
    writeQueue.current = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }, []);

  const announce = useCallback(
    (key: AnnouncementKey) => {
      setAnnouncement(t(key));
      setAnnouncementId((id) => id + 1);
    },
    [t],
  );

  const adopt = useCallback(
    (nextPins: readonly UserPinnedChannelResponse[], nextVersion: number) => {
      // 並行した GET や書き込みの応答が、すでに採用した版より遅れて届くことがある。
      // そのまま採用すると表示中の並びを巻き戻すので、古い版は捨てる。
      if (serverReady.current && nextVersion < versionRef.current) {
        return;
      }
      // 書き込みの応答や競合の本文でサーバーの状態を採用できたなら、取得の失敗は解消済み。
      setLoadError(null);
      const sorted = sortPins(nextPins);
      versionRef.current = nextVersion;
      serverReady.current = true;
      lastLiveSignature.current = pinsSignature(sorted);
      adoptCount.current += 1;
      setVersion(nextVersion);
      setPins(sorted);
    },
    [],
  );

  const beginPending = useCallback((key: string) => {
    const next = new Set(pendingRef.current);
    next.add(key);
    pendingRef.current = next;
    setPending(next);
  }, []);

  const endPending = useCallback((key: string) => {
    const next = new Set(pendingRef.current);
    next.delete(key);
    pendingRef.current = next;
    setPending(next);
  }, []);

  const fetchAuthoritative = useCallback(async (): Promise<ChannelPinsResponse | null> => {
    const requestGeneration = generation.current;
    const requestId = ++fetchSequence.current;
    try {
      const data = await api.getChannelPins();
      if (requestGeneration !== generation.current) return null;
      // 現在の世代の応答が返った時点で読み込みは終わる。古い版でも、
      // 書き込みなどで新しい状態を採用済みなら待機を続ける必要はない。
      setLoading(false);
      // 書き込みが先に採用した版より古い応答は採用しない。採用しなかった
      // 応答で成功順を進めないよう、ここで先に判定する。
      if (data.version < versionRef.current) return null;
      // 同一版では、後から始めた取得の結果を優先する。先発の成功が後から
      // 届いても、新しい archived や並びを巻き戻さない。より新しい版は採用する。
      if (data.version === versionRef.current && requestId < lastSuccessfulFetch.current) {
        return null;
      }
      lastSuccessfulFetch.current = Math.max(lastSuccessfulFetch.current, requestId);
      // エラーの解除も、実際に採用したときだけ `adopt` に任せる。
      adopt(data.pins, data.version);
      return data;
    } catch (cause) {
      if (requestGeneration !== generation.current) return null;
      // 新しい取得がすでに成功していれば、それより前に始まった取得の失敗は捨てる。
      if (requestId < lastSuccessfulFetch.current) return null;
      setLoading(false);
      setLoadError(localizeApiError(cause, t));
      return null;
    }
  }, [adopt, api, t]);

  // 言語を切り替えると `t` が変わり、取得関数の identity も変わる。効果の依存に
  // 含めると、購読のやり直しや保留中の書き込みの取りこぼしにつながるため ref で渡す。
  const fetchAuthoritativeRef = useRef(fetchAuthoritative);
  fetchAuthoritativeRef.current = fetchAuthoritative;

  // ユーザーが変わったら、前のユーザーのピンを残さず取り直す。
  useEffect(() => {
    generation.current += 1;
    versionRef.current = 0;
    serverReady.current = false;
    lastLiveSignature.current = null;
    fetchSequence.current = 0;
    lastSuccessfulFetch.current = 0;
    adoptCount.current = 0;
    pendingRef.current = new Set();
    setPending(new Set());
    setPins([]);
    setVersion(0);
    setLoadError(null);
    setWriteError(null);
    setAnnouncement("");
    writeQueue.current = Promise.resolve();
    if (!currentUserId || !collection) {
      setLoading(false);
      return;
    }
    setLoading(true);
    void fetchAuthoritativeRef.current();
  }, [collection, currentUserId]);

  const livePins = useMemo(
    () => (live.data ? sortPins(live.data as UserPinnedChannelRow[]) : null),
    [live.data],
  );
  const liveSignature = livePins ? pinsSignature(livePins) : null;

  // Electric の変更は、初期表示と、ほかの端末からの変更の検知に使う。版は Electric に
  // 無いため、変更があったときだけ GET で最新の版と並びを取り直す（書き込みの応答で
  // すでに採用していれば、同じ署名なので何もしない）。
  useEffect(() => {
    if (!livePins || !currentUserId || liveSignature === null) {
      return;
    }
    if (liveSignature === lastLiveSignature.current) {
      return;
    }
    if (!serverReady.current) {
      lastLiveSignature.current = liveSignature;
      setPins(livePins);
      setLoading(false);
      return;
    }
    if (pendingRef.current.size > 0) {
      return;
    }
    // 自分の書き込みのエコーは `adopt` が署名を更新済みなので、ここへは来ない。
    // 再取得が成功したときだけ `adopt` に署名を任せ、失敗時に取りこぼさない。
    const timer = setTimeout(() => {
      void fetchAuthoritativeRef.current();
    }, 250);
    return () => clearTimeout(timer);
  }, [currentUserId, livePins, liveSignature, pending]);

  const fail = useCallback(
    (
      cause: unknown,
      fallbackKey?: "channelPins.errors.conflict" | "channelPins.errors.reorderConflict",
    ) => {
      const message = isChannelPinsLimit(cause)
        ? t("channelPins.errors.limitExceeded")
        : channelPinsConflict(cause)
          ? t(fallbackKey ?? "channelPins.errors.conflict")
          : localizeApiError(cause, t);
      setWriteError(message);
      toast.error(message);
    },
    [t],
  );

  const pinChannel = useCallback(
    async (channelId: string) => {
      if (!currentUserId || !collection) return;
      const requestUserId = currentUserId;
      beginPending(channelId);
      setWriteError(null);
      const requestGeneration = generation.current;
      const stale = () =>
        isStalePinWrite(
          requestGeneration,
          generation.current,
          requestUserId,
          currentUserIdRef.current,
        );
      try {
        await enqueueWrite(async () => {
          for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
            // アカウント切り替え後は、順番待ちしていた書き込みを新しいユーザーとして送らない。
            if (stale()) return;
            try {
              const { data } = await api.pinChannel(channelId, versionRef.current);
              if (stale()) return;
              adopt(data.pins, data.version);
              announce("channelPins.announce.pinned");
              return;
            } catch (cause) {
              if (stale()) return;
              const conflict = channelPinsConflict(cause);
              if (!conflict) throw cause;
              adopt(conflict.pins, conflict.version);
              // ほかの端末が先にピンしていた。最新の状態をそのまま採用する。
              if (conflict.pins.some((pin) => pin.channel_id === channelId)) {
                announce("channelPins.announce.pinned");
                return;
              }
              if (attempt === MAX_ATTEMPTS - 1) throw cause;
            }
          }
        });
      } catch (cause) {
        if (stale()) return;
        fail(cause);
        if (channelPinsConflict(cause)) await fetchAuthoritative();
      } finally {
        if (!stale()) endPending(channelId);
      }
    },
    [
      adopt,
      announce,
      api,
      beginPending,
      collection,
      currentUserId,
      endPending,
      enqueueWrite,
      fail,
      fetchAuthoritative,
    ],
  );

  const unpinChannel = useCallback(
    async (channelId: string) => {
      if (!currentUserId || !collection) return;
      const requestUserId = currentUserId;
      beginPending(channelId);
      setWriteError(null);
      const requestGeneration = generation.current;
      const stale = () =>
        isStalePinWrite(
          requestGeneration,
          generation.current,
          requestUserId,
          currentUserIdRef.current,
        );
      try {
        await enqueueWrite(async () => {
          for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
            // アカウント切り替え後は、順番待ちしていた書き込みを新しいユーザーとして送らない。
            if (stale()) return;
            try {
              const { data } = await api.unpinChannel(channelId, versionRef.current);
              if (stale()) return;
              adopt(data.pins, data.version);
              announce("channelPins.announce.unpinned");
              return;
            } catch (cause) {
              if (stale()) return;
              const conflict = channelPinsConflict(cause);
              if (!conflict) throw cause;
              adopt(conflict.pins, conflict.version);
              // ほかの端末が先に外していた。最新の状態をそのまま採用する。
              if (!conflict.pins.some((pin) => pin.channel_id === channelId)) {
                announce("channelPins.announce.unpinned");
                return;
              }
              if (attempt === MAX_ATTEMPTS - 1) throw cause;
            }
          }
        });
      } catch (cause) {
        if (stale()) return;
        fail(cause);
        if (channelPinsConflict(cause)) await fetchAuthoritative();
      } finally {
        if (!stale()) endPending(channelId);
      }
    },
    [
      adopt,
      announce,
      api,
      beginPending,
      collection,
      currentUserId,
      endPending,
      enqueueWrite,
      fail,
      fetchAuthoritative,
    ],
  );

  const reorderPins = useCallback(
    async (nextOrder: string[], movedChannelId: string) => {
      if (!currentUserId || !collection || nextOrder.length === 0) return;
      const requestUserId = currentUserId;
      beginPending(REORDER_KEY);
      setWriteError(null);
      const requestGeneration = generation.current;
      const stale = () =>
        isStalePinWrite(
          requestGeneration,
          generation.current,
          requestUserId,
          currentUserIdRef.current,
        );
      const previousPins = sortPins(pinsRef.current);
      const versionBeforeOptimistic = versionRef.current;
      const signatureBeforeOptimistic = lastLiveSignature.current;
      const adoptionsBeforeOptimistic = adoptCount.current;
      const optimisticPins = reorderPinsLocally(previousPins, nextOrder);
      setPins(optimisticPins);
      let order = [...nextOrder];
      try {
        await enqueueWrite(async () => {
          for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
            // アカウント切り替え後は、順番待ちしていた書き込みを新しいユーザーとして送らない。
            if (stale()) return;
            try {
              const { data } = await api.reorderChannelPins(order, versionRef.current);
              if (stale()) return;
              adopt(data.pins, data.version);
              announce("channelPins.announce.moved");
              return;
            } catch (cause) {
              if (stale()) return;
              const conflict = channelPinsConflict(cause);
              if (!conflict) throw cause;
              adopt(conflict.pins, conflict.version);
              const latestActiveIds = conflict.pins
                .filter((pin) => !pin.archived)
                .map((pin) => pin.channel_id);
              if (!latestActiveIds.includes(movedChannelId)) {
                // ほかの端末で解除・アーカイブされたピンは、並び替えで復活させない。
                const archivedElsewhere = conflict.pins.some(
                  (pin) => pin.channel_id === movedChannelId && pin.archived,
                );
                announce(
                  archivedElsewhere
                    ? "channelPins.announce.reorderArchived"
                    : "channelPins.announce.reorderConflict",
                );
                return;
              }
              if (attempt === MAX_ATTEMPTS - 1) throw cause;
              // ほかの端末が追加したピンも残したまま、同じ移動をやり直す。
              order = mergeReorder(order, movedChannelId, latestActiveIds);
            }
          }
        });
      } catch (cause) {
        if (stale()) return;
        // 409 の本文や GET で権威状態を採用した後は、楽観更新前の一覧へ戻さない。
        // 同じ版で archived だけが変わる応答は版も署名も変えないため、
        // `adopt` の採用回数も合わせて確認する。
        const adoptedAuthoritative =
          adoptCount.current !== adoptionsBeforeOptimistic ||
          versionRef.current !== versionBeforeOptimistic ||
          lastLiveSignature.current !== signatureBeforeOptimistic;
        if (!adoptedAuthoritative) {
          setPins((current) =>
            pinsSignature(current) === pinsSignature(optimisticPins) ? previousPins : current,
          );
        }
        fail(cause, "channelPins.errors.reorderConflict");
        if (channelPinsConflict(cause)) await fetchAuthoritative();
      } finally {
        if (!stale()) endPending(REORDER_KEY);
      }
    },
    [
      adopt,
      announce,
      api,
      beginPending,
      collection,
      currentUserId,
      endPending,
      enqueueWrite,
      fail,
      fetchAuthoritative,
    ],
  );

  const refresh = useCallback(async () => {
    await fetchAuthoritative();
  }, [fetchAuthoritative]);

  const dismissError = useCallback(() => {
    setLoadError(null);
    setWriteError(null);
  }, []);

  const pinnedIds = useMemo(() => new Set(pins.map((pin) => pin.channel_id)), [pins]);

  const value = useMemo<PinnedChannelsValue>(
    () => ({
      available: Boolean(collection),
      pins,
      version,
      loading,
      pending,
      error: writeError ?? loadError,
      announcement,
      isPinned: (channelId: string) => pinnedIds.has(channelId),
      pinChannel,
      unpinChannel,
      reorderPins,
      refresh,
      dismissError,
    }),
    [
      announcement,
      collection,
      dismissError,
      loadError,
      loading,
      pending,
      pinChannel,
      pinnedIds,
      pins,
      refresh,
      reorderPins,
      unpinChannel,
      version,
      writeError,
    ],
  );

  return { value, announcementId };
}
