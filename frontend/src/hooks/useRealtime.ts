import { eq, isNull, useLiveQuery } from "@tanstack/react-db";
import { useMemo } from "react";

import { getCollections } from "../lib/electric/collections";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import type { ChannelRow, ChannelUserRow } from "../lib/electric/types";

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
 * 指定チャンネルのメッセージを投稿者名（users）と結合して購読する。
 * `channelId` が無いときは購読しない。
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

export interface MyChannelMemberships {
  /** 現在ユーザーが参加している channel_user 行。 */
  memberships: ChannelUserRow[];
  /** 参加中チャンネル ID の集合（一覧の参加バッジ判定用）。 */
  joinedChannelIds: ReadonlySet<string>;
  /** 開発ユーザーが解決できているか。false なら参加状態を表示しない。 */
  hasCurrentUser: boolean;
}

/**
 * 現在ユーザーが参加している channel_user を購読する。
 *
 * `channel_user` の Shape は `getCollections()` が 1 本だけ持つため、一覧と
 * メンバーパネルが同時に購読しても購読は増えない。`user_id` の索引を持たない
 * コレクションのため、絞り込みは live query の述語に押し込まず購読後に走査する。
 */
export function useMyChannelMemberships(): MyChannelMemberships {
  const { channelUsers, config } = getCollections();
  const currentUserId = config.devUserId;
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
  /** 非公開チャンネルは参加者のみ読める（direct の全件配信に対する UI 側ガード）。 */
  canRead: boolean;
  /** 追加は作成者か、公開チャンネルの既存メンバー（バックエンドと同じ条件）。 */
  canAddMember: boolean;
  /** メンバー購読の読み込み中。 */
  isLoading: boolean;
  /** メンバー購読の失敗。 */
  isError: boolean;
}

/**
 * チャンネルの権限判定をまとめる。
 *
 * 条件はバックエンドの認可と同じ: 非公開は参加者のみ閲覧、追加は作成者か公開の
 * 既存メンバー。direct モードの Shape は認可せず全件配信するため UI 側のガードが
 * 必要（proxy 切替後も同じ）。`channel` が未解決の間は購読せず、判定はすべて false。
 */
export function useChannelAccess(channel: ChannelRow | null): ChannelAccess {
  const { config } = getCollections();
  const membersQuery = useChannelMembers(channel?.id ?? null);
  const members = membersQuery.data ?? [];
  const currentUserId = config.devUserId;
  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;
  return {
    members,
    isMember,
    canRead: channel ? !channel.is_private || isMember : false,
    canAddMember: channel
      ? currentUserId === channel.created_user_id || (!channel.is_private && isMember)
      : false,
    isLoading: membersQuery.isLoading,
    isError: membersQuery.isError,
  };
}
