import { eq, isNull, useLiveQuery } from "@tanstack/react-db";
import { useEffect, useMemo, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import type { ChannelRow, ChannelUserRow } from "../lib/electric/types";
import { canAddChannelMember, canManageChannel } from "./channelPermissions";
import { useCurrentUser } from "./useCurrentUser";

/** 投稿者名の解決に使う全ユーザーを購読する。 */
export function useUsers() {
  const { users } = useCollections();
  return useLiveQuery((q) => q.from({ users }));
}

/** 論理削除されていないチャンネル一覧をチャンネル名の昇順で購読する。 */
export function useActiveChannels() {
  const { channels } = useCollections();
  return useLiveQuery((q) =>
    q
      .from({ channels })
      .where(({ channels }) => isNull(channels.deleted_at))
      .orderBy(({ channels }) => channels.channel_name, "asc"),
  );
}

export interface MessageFeedOptions {
  /** スレッドの返信を購読する場合のルート ID。未指定はチャンネル直下。 */
  parentId?: string | null;
  /** この時刻以降だけを表示する（30日窓）。 */
  since?: string;
}

/**
 * 指定チャンネルのルートメッセージ（`parent_id IS NULL`）を投稿者名（users）と
 * 結合して購読する。返信は同じ messages コレクションを `parent_id` で絞る。
 * `channelId` が無いときは購読しない。
 */
export function useMessageFeed(channelId: string | null, options: MessageFeedOptions = {}) {
  const { messages, users } = useCollections();
  const parentId = options.parentId ?? null;
  const since = options.since;
  return useLiveQuery((q) =>
    channelId
      ? messageFeedQuery(q, { messages: messages(channelId), users }, { parentId, since })
      : undefined,
  );
}

/** 指定チャンネルの参加メンバー（channel_user）を購読する。 */
export function useChannelMembers(channelId: string | null) {
  const { channelUsers } = useCollections();
  return useLiveQuery((q) =>
    channelId
      ? q.from({ channelUsers }).where(({ channelUsers }) => eq(channelUsers.channel_id, channelId))
      : undefined,
  );
}

/** チャンネル内のメッセージへのリアクションを購読する。 */
export function useMessageReactions(channelId: string) {
  const { reactions } = useCollections();
  return useLiveQuery((q) => q.from({ reactions: reactions(channelId) }));
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
    isLoading: membershipsQuery.isLoading,
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
    isMembershipPending: !!channel?.is_private && (membersQuery.isLoading || currentUserLoading),
    isError: membersQuery.isError,
  };
}
