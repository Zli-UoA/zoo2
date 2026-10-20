import { eq, isNull, useLiveQuery } from "@tanstack/react-db";
import { useMemo } from "react";

import { getCollections } from "../lib/electric/collections";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import type { ChannelRow, ChannelUserRow } from "../lib/electric/types";
import { canAddChannelMember, canManageChannel } from "./channelPermissions";
import { useCurrentUser } from "./useCurrentUser";

/** 投稿者名の解決に使う全ユーザーを購読する。 */
export function useUsers() {
  const { users } = getCollections();
  return useLiveQuery((q) => q.from({ users }));
}

/** 論理削除されていないチャンネル一覧を購読する。 */
export function useActiveChannels() {
  const { channels } = getCollections();
  return useLiveQuery((q) =>
    q
      .from({ channels })
      .where(({ channels }) => isNull(channels.deleted_at))
      .orderBy(({ channels }) => channels.created_at),
  );
}

/**
 * 指定チャンネルのルートメッセージ（`parent_id IS NULL`）を投稿者名（users）と
 * 結合して購読する。返信は `useThreadFeed` が同じ messages コレクションを
 * `parent_id` で絞って購読する。`channelId` が無いときは購読しない。
 */
export function useMessageFeed(channelId: string | null) {
  const { messages, users } = getCollections();
  return useLiveQuery((q) =>
    channelId ? messageFeedQuery(q, { messages: messages(channelId), users }) : undefined,
  );
}

/** 指定チャンネルの参加メンバー（channel_user）を購読する。 */
export function useChannelMembers(channelId: string | null) {
  const { channelUsers } = getCollections();
  return useLiveQuery((q) =>
    channelId
      ? q.from({ channelUsers }).where(({ channelUsers }) => eq(channelUsers.channel_id, channelId))
      : undefined,
  );
}

/** チャンネル内のメッセージへのリアクションを購読する。 */
export function useMessageReactions(channelId: string) {
  const { reactions } = getCollections();
  return useLiveQuery((q) => q.from({ reactions: reactions(channelId) }));
}

export interface MyChannelMemberships {
  /** 現在ユーザーが参加している channel_user 行。 */
  memberships: ChannelUserRow[];
  /** 参加中チャンネル ID の集合（一覧の参加バッジ判定用）。 */
  joinedChannelIds: ReadonlySet<string>;
  /** 現在ユーザーが解決できているか。false なら参加状態を表示しない。 */
  hasCurrentUser: boolean;
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
  const { channelUsers } = getCollections();
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
  return { memberships, joinedChannelIds, hasCurrentUser: currentUserId !== null };
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
