import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { useMessageReactions } from "../../../hooks/useRealtime";
import type { WriteRunner } from "../../../hooks/useWriteRunner";
import { useCollections } from "../../../lib/electric/collections-context";
import { reactionKey } from "../../../lib/emoji/kitchen";
import {
  groupReactions,
  reactionUserLabel,
  type ReactionGroup,
} from "../../../lib/emoji/reactions";
import { localizeApiError } from "../../../lib/i18n/apiError";
import { nowIso } from "../../../lib/time";

export interface ChannelReactionUser {
  id: string;
  display_name: string;
  user_name: string;
}

export interface UseChannelReactionsOptions {
  channelId: string;
  currentUserId: string | null;
  users: readonly ChannelReactionUser[] | undefined;
  usersLoading?: boolean;
  readOnly?: boolean;
  isBlockedMessage?: (messageId: string) => boolean;
  runWrite?: WriteRunner["run"];
}

/**
 * チャンネル・キャッチアップで共有するリアクションの購読・集計・ピッカー制御・楽観更新。
 */
export function useChannelReactions({
  channelId,
  currentUserId,
  users,
  usersLoading = false,
  readOnly = false,
  isBlockedMessage,
  runWrite,
}: UseChannelReactionsOptions) {
  const { t } = useTranslation();
  const { messages, reactions } = useCollections();
  const messagesCollection = messages(channelId);
  const reactionsCollection = reactions(channelId);
  const reactionsQuery = useMessageReactions(channelId);
  const reactionRows = reactionsQuery.data ?? [];

  const reactionGroups = useMemo<Map<string, ReactionGroup[]>>(() => {
    const names = Object.fromEntries(
      (users ?? []).map((user) => [user.id, reactionUserLabel(user)]),
    );
    const groups = groupReactions(reactionRows, currentUserId);
    for (const groupList of groups.values()) {
      for (const reaction of groupList) {
        reaction.userNames = { ...reaction.userNames, ...names };
        reaction.usersLoading = usersLoading;
      }
    }
    return groups;
  }, [reactionRows, currentUserId, users, usersLoading]);

  const { catalog: kitchenCatalog } = useKitchenCatalog(
    reactionRows.some((row) => row.emoji.startsWith("kitchen:")),
  );

  const canReact =
    !readOnly && !!currentUserId && !reactionsQuery.isLoading && !reactionsQuery.isError;

  const isBlocked = useCallback(
    (messageId: string) => {
      if (isBlockedMessage) return isBlockedMessage(messageId);
      const message = messagesCollection.get(messageId);
      const rootId = message?.parent_id ?? messageId;
      return !!messagesCollection.get(rootId)?.thread_archive_at;
    },
    [isBlockedMessage, messagesCollection],
  );

  const [reactionMessageId, setReactionMessageId] = useState<string | null>(null);
  const pendingReactions = useRef(new Set<string>());

  const openReactionPicker = useCallback(
    (messageId: string) => {
      if (!canReact || isBlocked(messageId) || messagesCollection.get(messageId)?.deleted_at) {
        return;
      }
      setReactionMessageId(messageId);
    },
    [canReact, isBlocked, messagesCollection],
  );

  const closeReactionPicker = useCallback(() => {
    setReactionMessageId(null);
  }, []);

  useEffect(() => {
    setReactionMessageId(null);
  }, [channelId]);

  useEffect(() => {
    if (!reactionMessageId) return;
    if (
      !canReact ||
      isBlocked(reactionMessageId) ||
      !!messagesCollection.get(reactionMessageId)?.deleted_at
    ) {
      setReactionMessageId(null);
    }
  });

  const executeWrite = useCallback(
    async (action: Parameters<WriteRunner["run"]>[0]) => {
      if (runWrite) {
        await runWrite(action);
        return;
      }
      try {
        const transaction = await action();
        await transaction.when("settled");
      } catch (cause) {
        toast.error(t("common.writeError"), {
          description: localizeApiError(cause, t),
          id: "message-write-error",
        });
      }
    },
    [runWrite, t],
  );

  const handleReaction = useCallback(
    async (messageId: string, emoji: string) => {
      if (
        !canReact ||
        !currentUserId ||
        isBlocked(messageId) ||
        !messagesCollection.get(messageId) ||
        messagesCollection.get(messageId)?.deleted_at
      ) {
        return;
      }
      const key = reactionKey(emoji);
      const pendingKey = `${messageId}:${key}`;
      if (pendingReactions.current.has(pendingKey)) {
        return;
      }
      pendingReactions.current.add(pendingKey);
      try {
        await executeWrite(() => {
          const own = Array.from(reactionsCollection.values()).find(
            (row) =>
              row.message_id === messageId &&
              row.user_id === currentUserId &&
              reactionKey(row.emoji) === key,
          );
          return own
            ? reactionsCollection.delete(own.id)
            : reactionsCollection.insert({
                id: crypto.randomUUID(),
                message_id: messageId,
                user_id: currentUserId,
                emoji,
                created_at: nowIso(),
              });
        });
      } finally {
        pendingReactions.current.delete(pendingKey);
      }
    },
    [canReact, currentUserId, isBlocked, messagesCollection, reactionsCollection, executeWrite],
  );

  return {
    reactionGroups,
    kitchenCatalog,
    canReact,
    isLoading: reactionsQuery.isLoading,
    isError: reactionsQuery.isError,
    reactionMessageId,
    openReactionPicker,
    closeReactionPicker,
    handleReaction,
  };
}
