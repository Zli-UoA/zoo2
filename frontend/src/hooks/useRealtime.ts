import { useOptimisticDisplay } from "./useOptimisticDisplay";
import { eq, isNull, useLiveInfiniteQuery, useLiveQuery } from "@tanstack/react-db";
import { useEffect, useMemo, useRef, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import { MESSAGE_PAGE_SIZE } from "../lib/electric/messageOrder";
import type { ChannelRow, ChannelUserRow } from "../lib/electric/types";
import { clearFeedCache, loadFeedCache, saveFeedCache } from "../lib/offline/feedCache";
import { useOfflineStatus } from "../lib/offline/offline-context";
import type { FeedMessage as FeedRow } from "../components/chat/messages/messageFormat";
import { canAddChannelMember, canManageChannel } from "./channelPermissions";
import { useCurrentUser } from "./useCurrentUser";

/** 投稿者名の解決に使う全ユーザーを購読する。 */
export function useUsers() {
  const { users } = useCollections();
  const query = useLiveQuery((q) => q.from({ users }));
  return {
    ...query,
    isLoading: query.isLoading && !(query.isPersistedReady && query.data?.length),
  };
}

/** 論理削除されていないチャンネル一覧をチャンネル名の昇順で購読する。 */
export function useActiveChannels() {
  const { channels } = useCollections();
  const query = useLiveQuery((q) =>
    q
      .from({ channels })
      .where(({ channels }) => isNull(channels.deleted_at))
      .orderBy(({ channels }) => channels.channel_name, "asc"),
  );
  // 保存済みの一覧を復元できたら、初回のネットワーク同期は裏で続ける。
  return {
    ...query,
    // キャッシュにない対象の不存在判定は初回同期まで保留する。
    isInitialSyncLoading: query.isLoading,
    isLoading: query.isLoading && !(query.isPersistedReady && query.data?.length),
  };
}

export interface MessageFeedOptions {
  /** スレッドの返信を購読する場合のルート ID。未指定はチャンネル直下。 */
  parentId?: string | null;
}

/**
 * 指定チャンネルのルートメッセージ（`parent_id IS NULL`）を投稿者名（users）と
 * 結合して購読する。返信は同じ messages コレクションを `parent_id` で絞る。
 *
 * 端末に同期するのは、画面に出す範囲 (新しい方から `MESSAGE_PAGE_SIZE` 件ずつ) だけ。`loadOlder()` で、
 * 1 ページ分、古い方へ範囲を広げる (Electric の subset snapshot で、そのページだけを取る)。`data` は古い順。
 */
export function useMessageFeed(channelId: string, options: MessageFeedOptions = {}) {
  const { messages, users } = useCollections();
  const { isOnline, namespace } = useOfflineStatus();
  const parentId = options.parentId ?? null;
  const query = useLiveInfiniteQuery(
    (q) => messageFeedQuery(q, { messages: messages(channelId), users }, { parentId }),
    { pageSize: MESSAGE_PAGE_SIZE },
  );
  // クエリは新しい順。画面は古い順に並べる。
  const oldestFirst = useMemo(() => [...(query.data ?? [])].reverse(), [query.data]);

  // オフラインで再読み込みすると、on-demand のコレクションは保存済みの行を使えない。
  // 最新ページの控え (IndexedDB) を、読み込めた間だけ、代わりに表示する。
  const cacheable = parentId === null;
  const live = oldestFirst as FeedRow[];
  const liveReady = !query.isLoading && live.length > 0;
  const [cached, setCached] = useState<{ channelId: string; rows: FeedRow[] | null } | null>(null);
  // `navigator.onLine` は、サーバーに届かない状態でも真になる。そのため、オフラインのときだけでなく、
  // ライブの行が届くまで (読み込み中・失敗) は、控えを表示しておく。
  const wantCache =
    cacheable && live.length === 0 && (!isOnline || query.isLoading || query.isError);
  useEffect(() => {
    if (!wantCache) return;
    let cancelled = false;
    void loadFeedCache<FeedRow>(namespace, channelId).then((entry) => {
      if (!cancelled) setCached({ channelId, rows: entry?.rows ?? null });
    });
    return () => {
      cancelled = true;
    };
  }, [wantCache, namespace, channelId]);
  const cachedRows = wantCache && cached?.channelId === channelId ? cached.rows : null;

  // オンラインで読み込めたら、最新ページを控える (書き込みが続くときのために、間引く)。
  const latest = useRef(live);
  latest.current = live;
  useEffect(() => {
    if (!cacheable || !isOnline || !liveReady) return;
    const timer = setTimeout(
      () => void saveFeedCache(namespace, channelId, latest.current, MESSAGE_PAGE_SIZE),
      1_000,
    );
    return () => clearTimeout(timer);
  }, [cacheable, isOnline, liveReady, namespace, channelId, query.data]);

  // 正常に取得できた結果が空 (全件が削除された) なら、控えも消す。読み込み中の空とは区別する。
  const loadedEmpty = query.isReady && !query.isLoading && live.length === 0;
  useEffect(() => {
    if (!cacheable || !isOnline || !loadedEmpty) return;
    const timer = setTimeout(() => void clearFeedCache(namespace, channelId), 1_000);
    return () => clearTimeout(timer);
  }, [cacheable, isOnline, loadedEmpty, namespace, channelId]);

  const data = useOptimisticDisplay(channelId, cachedRows ?? oldestFirst);
  const fromCache = cachedRows !== null;
  return {
    ...query,
    data,
    isInitialSyncLoading: query.isLoading && !(fromCache && !isOnline),
    // オンラインで控えを出している間は、読み込み中のまま (最初のページが届くまで、書き込みを待たせる)。
    isLoading:
      query.isLoading &&
      !(query.isPersistedReady && query.data?.length) &&
      !(fromCache && !isOnline),
    loadOlder: query.fetchNextPage,
    // 控えには、古いページがない。
    hasOlder: fromCache ? false : query.hasNextPage,
    isLoadingOlder: query.isFetchingNextPage,
  };
}

/** 指定チャンネルの参加メンバー（channel_user）を購読する。 */
export function useChannelMembers(channelId: string | null) {
  const { channelUsers } = useCollections();
  const query = useLiveQuery((q) =>
    channelId
      ? q.from({ channelUsers }).where(({ channelUsers }) => eq(channelUsers.channel_id, channelId))
      : undefined,
  );
  return {
    ...query,
    isInitialSyncLoading: query.isLoading,
    isLoading: query.isLoading && !(query.isPersistedReady && query.data?.length),
  };
}

/** チャンネル内のメッセージへのリアクションを購読する。 */
export function useMessageReactions(channelId: string) {
  const { reactions } = useCollections();
  const native = useLiveQuery((q) => q.from({ reactions: reactions(channelId) }));
  // Display reactions, including federated actors, arrive with messages.display_data.
  // Keep this collection for optimistic writes and legacy cached messages only.
  return native;
}

export interface WorkspaceAdminIds {
  /** ワークスペース管理者の user_id 集合。 */
  adminIds: ReadonlySet<string>;
  /** 取得中か。失敗時も false になり、その場合はキック可否をバックエンドに委ねる。 */
  isLoading: boolean;
}

/**
 * キック可否の UI 判定に使うワークスペース管理者の一覧を取得する。
 *
 * 管理者ロールは `users` / `channel_user` の Shape に含まれないため、
 * `GET /api/users/admins` の軽量エンドポイントから一度だけ取得する。Shape を
 * 増やさず、管理者をキック対象として表示しないための最小限の情報にとどめる。
 */
export function useWorkspaceAdminIds(): WorkspaceAdminIds {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [adminIds, setAdminIds] = useState<ReadonlySet<string>>(() => new Set());
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      client
        .get<{ user_ids: string[] }>("/api/users/admins")
        .then((response) => {
          if (cancelled) return;
          setAdminIds(new Set(response.user_ids));
          setIsLoading(false);
        })
        .catch((cause) => {
          if (cancelled) return;
          console.warn("ワークスペース管理者の取得に失敗しました", cause);
          setAdminIds(new Set());
          setIsLoading(false);
        });
    void load();
    window.addEventListener("zoo:admins-changed", load);
    return () => {
      window.removeEventListener("zoo:admins-changed", load);
      cancelled = true;
    };
  }, [client]);

  return { adminIds, isLoading };
}

export interface MyChannelMemberships {
  /** キャッシュにない投稿先を判定するための初回同期状態。 */
  isInitialSyncLoading: boolean;
  /** 現在ユーザーが参加している channel_user 行。 */
  memberships: ChannelUserRow[];
  /** 参加中チャンネル ID の集合（一覧の参加バッジ判定用）。 */
  joinedChannelIds: ReadonlySet<string>;
  /** 現在ユーザーが解決できているか。false なら参加状態を表示しない。 */
  hasCurrentUser: boolean;
  /** チャンネル参加状態の初回読み込み中。 */
  isLoading: boolean;
}

/**
 * 現在ユーザーが参加している channel_user を購読する。
 *
 * 現在ユーザーは `useCurrentUser()`（dev は `VITE_DEV_USER_ID`、OIDC は
 * `/api/users/me`）から取る。`channel_user` の Shape は `getCollections()` が
 * 1 本だけ持つため、一覧とメンバーパネルが同時に購読しても購読は増えない。
 * `user_id` の索引を持たないコレクションのため、絞り込みは live query の述語に
 * 押し込まず購読後に走査する。
 */
export function useMyChannelMemberships(): MyChannelMemberships {
  const { channelUsers } = useCollections();
  const { currentUserId } = useCurrentUser();
  const membershipsQuery = useLiveQuery((q) => q.from({ channelUsers }));
  const memberships = useMemo(
    () => (membershipsQuery.data ?? []).filter((row) => row.user_id === currentUserId),
    [membershipsQuery.data, currentUserId],
  );
  const joinedChannelIds = useMemo(
    () => new Set(memberships.map((row) => row.channel_id)),
    [memberships],
  );
  return {
    memberships,
    joinedChannelIds,
    hasCurrentUser: currentUserId !== null,
    isInitialSyncLoading: membershipsQuery.isLoading,
    isLoading:
      membershipsQuery.isLoading && !(membershipsQuery.isPersistedReady && memberships.length),
  };
}

export interface ChannelAccess {
  /** チャンネルの参加メンバー（一覧表示にも使う）。 */
  members: ChannelUserRow[];
  /** 現在ユーザーが参加しているか。 */
  isMember: boolean;
  /** 現在ユーザーがチャンネル作成者か。 */
  isCreator: boolean;
  /** 現在ユーザーがワークスペース管理者か。 */
  isAdmin: boolean;
  /** 非公開チャンネルは参加者のみ読める（direct の全件配信に対する UI 側ガード）。 */
  canRead: boolean;
  /** 追加は管理者か作成者、公開チャンネルの既存メンバー（バックエンドと同じ条件）。 */
  canAddMember: boolean;
  /** アーカイブ・キック・強制削除（管理者か作成者）。 */
  canManageChannel: boolean;
  /** アーカイブ済み（読み取り専用）か。 */
  isArchived: boolean;
  /** メンバー購読の読み込み中（ヘッダーの件数表示用）。 */
  isLoading: boolean;
  /**
   * 非公開チャンネルで、参加判定に必要な購読（メンバー / 現在ユーザー）が
   * 揃うまで保留する。解決前に「非公開です」を出すのを避ける。
   */
  isMembershipPending: boolean;
  /** メンバー購読の失敗。 */
  isError: boolean;
}

/**
 * チャンネルの権限判定をまとめる。
 *
 * 条件はバックエンドの認可と同じ: 非公開は参加者のみ閲覧、追加は管理者か作成者
 * または公開の既存メンバー、管理は管理者か作成者。direct モードの Shape は認可
 * せず全件配信するため UI 側のガードが必要（proxy 切替後も同じ）。現在ユーザーと
 * `is_admin` は `useCurrentUser()` から取る。`channel` が未解決の間はメンバーを
 * 購読せず、判定はすべて false。
 */
export function useChannelAccess(channel: ChannelRow | null): ChannelAccess {
  const { currentUserId, isLoading: currentUserLoading, isAdmin } = useCurrentUser();
  const membersQuery = useChannelMembers(channel?.id ?? null);
  const members = membersQuery.data ?? [];
  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;
  const isCreator = !!channel && currentUserId === channel.created_user_id;
  return {
    members,
    isMember,
    isCreator,
    isAdmin,
    canRead: channel ? !channel.is_private || isMember : false,
    canAddMember: channel
      ? canAddChannelMember({
          isAdmin,
          isCreator,
          isPrivate: channel.is_private,
          isMember,
        })
      : false,
    canManageChannel: channel ? canManageChannel({ isAdmin, isCreator }) : false,
    isArchived: !!channel?.archive_at,
    isLoading: membersQuery.isLoading,
    isMembershipPending:
      !!channel?.is_private &&
      (currentUserLoading || (!isMember && membersQuery.isInitialSyncLoading)),
    isError: membersQuery.isError,
  };
}
