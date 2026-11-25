import {
  createCollection,
  createLiveQueryCollection,
  localOnlyCollectionOptions,
  useLiveInfiniteQuery,
  useLiveQuery,
} from "@tanstack/react-db";
import { useCallback, useMemo, useSyncExternalStore } from "react";

import { useCollections } from "../lib/electric/collections-context";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import { MESSAGE_PAGE_SIZE } from "../lib/electric/messageOrder";
import type { MessageRow } from "../lib/electric/types";
import { useOptimisticDisplay } from "./useOptimisticDisplay";

const NO_ROWS: never[] = [];

/**
 * 指定ルートへの返信を、メイン一覧と同じ messages コレクションの live query で購読する。
 * スレッド専用の Shape / collection は作らない。メイン一覧と同じく、新しい方から
 * `MESSAGE_PAGE_SIZE` 件ずつ (`loadOlder()` で古い方へ)。`data` は古い順。
 */
export function useThreadFeed(channelId: string, rootId: string | null) {
  const { messages, users } = useCollections();
  const empty = useMemo(
    () =>
      createCollection(
        localOnlyCollectionOptions<MessageRow>({
          getKey: (row) => row.id,
          initialData: [],
        }),
      ),
    [],
  );
  const query = useLiveInfiniteQuery(
    (q) =>
      messageFeedQuery(
        q,
        { messages: rootId ? messages(channelId) : empty, users },
        { parentId: rootId },
      ),
    { pageSize: MESSAGE_PAGE_SIZE },
  );
  const oldestFirst = useMemo(() => [...(query.data ?? [])].reverse(), [query.data]);
  const data = useOptimisticDisplay(rootId ? channelId : null, rootId ? oldestFirst : NO_ROWS);
  return {
    ...query,
    data,
    loadOlder: query.fetchNextPage,
    hasOlder: query.hasNextPage,
    isLoadingOlder: query.isFetchingNextPage,
  };
}

/**
 * ID で 1 件のメッセージを取る (スレッドの親、通知やリンクの遷移先)。メイン一覧の範囲とは関係なく、
 * Electric の subset snapshot で、その行だけを取る。削除済みの行も返す (`deleted_at`)。
 */
export function useMessageById(channelId: string | null, messageId: string | null) {
  const { messages, users } = useCollections();
  const query = useLiveQuery((q) =>
    channelId && messageId
      ? messageFeedQuery(
          q,
          { messages: messages(channelId), users },
          { parentId: null, messageId, includeDeleted: true },
        )
      : undefined,
  );
  const data = useOptimisticDisplay(channelId && messageId ? channelId : null, query.data);
  return { ...query, data };
}

type MessageFeedSources = Parameters<typeof messageFeedQuery>[1];

/**
 * リンク先のメッセージを取る、共有のライブクエリ。リンクの短い表示・埋め込み・別の投稿など、同じメッセージを
 * 指すものが、1 つの問い合わせ（Electric の subset snapshot）を分け合う。`useLiveQuery` に問い合わせを
 * 渡すと、呼び出しごとに別の取得になり、同じメッセージへのリンクの数だけ要求が増える。
 * 鍵は、チャンネルの `messages` コレクション（アカウントごとのコレクション群に属する）と ID。
 */
const linkedMessageQueries = new WeakMap<
  object,
  Map<string, ReturnType<typeof createLinkedQuery>>
>();

function createLinkedQuery(sources: MessageFeedSources, messageId: string) {
  return createLiveQueryCollection({
    query: (q) => messageFeedQuery(q, sources, { parentId: null, messageId, includeDeleted: true }),
    // 最初の購読者が付くまで、取得しない。
    startSync: false,
  });
}

function linkedMessageQuery(sources: MessageFeedSources, messageId: string) {
  let byId = linkedMessageQueries.get(sources.messages);
  if (!byId) {
    byId = new Map();
    linkedMessageQueries.set(sources.messages, byId);
  }
  const existing = byId.get(messageId);
  // 購読者がいなくなって片付いた問い合わせは、作り直す。失敗した（`error`）問い合わせは、片付くまで使い続ける。
  // ここは描画のたびに呼ばれる。失敗の原因（チャンネルの `messages` の同期の失敗など）が残っていると、作り直した
  // 問い合わせもすぐ失敗し、描画のたびに購読と要求が増える。作り直しても、`messages` が失敗のままなら
  // 取り直さない。`messages` はアプリで共有するので、ここでは片付けない。取り直すのは、ページの再読み込みか、
  // 使う画面がなくなって TanStack DB が片付けた（`gcTime` の既定は 5 分）あと。
  if (existing && existing.status !== "cleaned-up") return existing;
  const created = createLinkedQuery(sources, messageId);
  byId.set(messageId, created);
  const forget = ({ status }: { status: string }) => {
    if (status === "cleaned-up" && byId.get(messageId) === created) byId.delete(messageId);
  };
  created.on("status:change", forget);
  return created;
}

/**
 * メッセージリンクの埋め込み用に、ID で 1 件のメッセージを取る。{@link useMessageById} と同じ問い合わせだが、
 * 楽観更新の重ねがけ（`useOptimisticDisplay`）はしない。それは、チャンネルのリアクションの Shape まで
 * 購読するため、リンク先のチャンネルごとに余計な同期が増える。埋め込みはリアクションを表示しない。
 * `channelId` が `null` のときは、何も取得しない。
 */
export function useLinkedMessageRows(channelId: string | null, messageId: string | null) {
  const { messages, users } = useCollections();
  const query =
    channelId && messageId
      ? linkedMessageQuery({ messages: messages(channelId), users }, messageId)
      : null;
  return useLiveQuery(() => query);
}

/**
 * この端末が読み込んでいる、そのチャンネルの行に、暗号文のある行があるか。暗号化の格下げの検出
 * （`encryptionSendState` の `hasCiphertext`、`MessagePane` は表示中の行で判定する）に使う。
 *
 * 行を取る要求は出さない。メッセージリンクの埋め込みは対象の 1 件しか取らないので、同じチャンネルの別の行は、
 * チャンネルを開いて読み込んだ分だけが見える。変更の購読は、行を取らずに変更だけを受ける
 * （`useOptimisticDisplay` と同じ。on-demand のコレクションで、全行を取る要求にしないため）。
 *
 * **注意**: `subscribeChanges` は、`includeInitialState: false` でも、そのコレクションの同期を始める
 * （初期状態の通知を止めるだけ）。そのチャンネルの `messages` を、ほかの理由で同期するときだけ使う
 * （メッセージを取得する {@link useLinkedMessageRows} と同じチャンネルなど）。これだけのために
 * 同期を増やさない。`channelId` が `null` のときは、見ない。
 */
export function useChannelHasCiphertext(channelId: string | null): boolean {
  const { messages } = useCollections();
  const collection = channelId ? messages(channelId) : null;
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (!collection) return () => {};
      const handle = collection.subscribeChanges(onChange, { includeInitialState: false });
      return () => handle.unsubscribe();
    },
    [collection],
  );
  return useSyncExternalStore(
    subscribe,
    () => {
      if (!collection) return false;
      for (const row of collection.values()) if (row.ciphertext != null) return true;
      return false;
    },
    () => false,
  );
}

/** スレッドの親を、メイン一覧の範囲とは関係なく取る。 */
export function useThreadRoot(channelId: string | null, rootId: string | null) {
  return useMessageById(channelId, rootId);
}
