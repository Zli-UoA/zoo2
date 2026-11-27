import {
  THREADS_CHANGED,
  THREAD_REPLY_REDIRECTED,
  type ThreadReplyRedirect,
} from "../../../lib/electric/threads";
import { isThreadJoinRequired, useThreadMembership } from "../../../hooks/useThreadMembership";
import { refreshAppSurfaces } from "../../../lib/appSurfaceEvents";
import { ApiError, createConfiguredApiClient } from "../../../lib/api/client";
import { forgetForwardIds } from "../../../lib/api/forwards";
import { DEFAULT_CONFIG } from "../../../lib/config";
import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { useChannelCommands } from "../../../hooks/useChannelCommands";
import { useCurrentUser } from "../../../hooks/useCurrentUser";
import {
  useAttachmentUpload,
  useEncryptedAttachmentUpload,
  useMessageAttachments,
} from "../../../hooks/useAttachments";
import {
  ForwardedMessagesProvider,
  useForwardedMessages,
} from "../../../hooks/useForwardedMessages";
import type { AttachmentInfo } from "@zoo/crypto-wasm";

import type { UploadedAttachment } from "../../../lib/api/attachments";
import type {
  CreateMessageRequest,
  MessageResponse,
  ResolvedForwardMessage,
} from "../../../lib/api/types";
import {
  useActiveChannels,
  useChannelMembers,
  useUsers,
  useMessageFeed,
  useMyChannelMemberships,
} from "../../../hooks/useRealtime";
import { useMessageById, useThreadFeed, useThreadRoot } from "../../../hooks/useThread";
import { useWriteRunner } from "../../../hooks/useWriteRunner";
import { groupAttachmentsByMessage } from "../../../lib/attachments";
import { useCollections } from "../../../lib/electric/collections-context";
import type { AttachmentRow } from "../../../lib/electric/types";
import type { ChannelUserWriteMetadata } from "../../../lib/electric/writes";
import { useOfflineStatus } from "../../../lib/offline/offline-context";
import { nowIso } from "../../../lib/time";
import { useLocale } from "../../i18n/LocaleProvider";
import { useShell } from "../../layout/shell-context";
import { ConfirmActionDialog } from "../../ui/confirm-action-dialog";
import { useTyping } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { ForwardMessageDialog, buildForwardDestinations } from "./ForwardMessageDialog";
import { JoinPrompt } from "./JoinPrompt";
import { MessageList } from "./MessageList";
import type { FeedMessage } from "./messageFormat";
import { ReactionPicker } from "./ReactionPicker";
import { ThreadPane } from "./ThreadPane";
import { ThreadPaneStatus } from "./ThreadPaneStatus";
import { useChannelReactions } from "./useChannelReactions";
import { EncryptionRecipientsNotice } from "./EncryptionRecipientsNotice";
import { encryptedMentions } from "./mentions";
import { useMessageDecryptions } from "../../../hooks/useMessageDecryptions";
import {
  EncryptedSendError,
  editAlreadyApplied,
  editEncryptedMessage,
  sendEncryptedMessage,
  type ChannelCryptoDeps,
} from "../../../lib/e2ee/channelSend";
import { useE2ee } from "../../../lib/e2ee/e2ee-context";
import { useEncryptedDisplay } from "../../../hooks/useEncryptedDisplay";
import { apiErrorCode } from "../../../lib/e2ee/megolmApi";
import {
  isPlaintextBeforeEncryption,
  MessageDecryptionsProvider,
  firstEncryptedAt,
  withUnencryptedRows,
} from "../../../lib/e2ee/messageDecryption";
import type { RecipientSummary } from "../../../lib/e2ee/store/megolm";
import { localizeApiError } from "../../../lib/i18n/apiError";
import { apiPath } from "../../../lib/api/path";

/** 通知・リンクの対象を含むまで、古い方へ広げるページ数の上限 (1 ページは `MESSAGE_PAGE_SIZE` 件)。 */
const MAX_REVEAL_PAGES = 200;

function JoinToPostPrompt({
  onJoin,
  joining,
  disabled,
}: {
  onJoin: () => void;
  joining: boolean;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  return (
    <JoinPrompt
      description={t("messages.joinToPostDescription")}
      actionLabel={t("messages.joinToPost")}
      onAction={onJoin}
      pending={joining}
      disabled={disabled}
    />
  );
}

/**
 * チャンネルのメッセージ表示・送信・編集・論理削除に加え、リアクションと
 * スレッドを扱う。
 *
 * データ層は `useMessageFeed`（messages ⋈ users の live query）と
 * `useWriteRunner`（楽観更新の確定待ち）をそのまま使い、表示だけを作り込む。
 * スレッドも同じ messages コレクションを `parent_id` で絞るだけで、専用の
 * Shape / コレクションは作らない。
 *
 * 未参加の公開チャンネルでは Composer の代わりに参加導線を出し、参加後に
 * 通常の Composer へ戻す（`isMember` は `useChannelAccess` の判定）。
 */
export function MessagePane({
  channelId,
  isMember,
  canModerate = false,
  readOnly = false,
  postingForbidden = false,
  encrypted = false,
  membershipEpoch,
  channelName,
  targetMessageId,
}: {
  channelId: string;
  isMember: boolean;
  canModerate?: boolean;
  readOnly?: boolean;
  /** 投稿権限がない（`posting_policy=managers` で管理者・作成者ではない）。閲覧はでき、入力欄だけを理由付きで止める。 */
  postingForbidden?: boolean;
  /** 暗号化チャンネル。初期版では添付できないので、添付ボタンを出さない（docs/e2ee.md）。 */
  encrypted?: boolean;
  /** 暗号化チャンネルの Channels Shape の `membership_epoch`（int8 なので文字列）。 */
  membershipEpoch?: string | null;
  /** チャンネル名（`edm:` の暗号化 DM を、`encrypted` の値にかかわらず暗号化として扱うため）。 */
  channelName?: string;
  targetMessageId?: string;
}) {
  const { t } = useTranslation();
  const { messages, channelUsers, config } = useCollections();
  const appClient = useMemo(() => createConfiguredApiClient(config), [config]);
  const usersQuery = useUsers();
  const mentionUsers = (usersQuery.data ?? []).filter(
    (user) => !user.is_bot || user.bot_mentionable,
  );
  const members = useChannelMembers(channelId).data;
  // 復号した `mentions.user_ids` を本文の `@name` と突き合わせるための、ユーザー ID からユーザー名への表。
  const mentionUserNames = useMemo(
    () => new Map((usersQuery.data ?? []).map((user) => [user.id, user.user_name])),
    [usersQuery.data],
  );
  const mentionMemberIds = useMemo(
    () => new Set((members ?? []).map((member) => member.user_id)),
    [members],
  );
  const messagesCollection = messages(channelId);
  // 新しい方から `MESSAGE_PAGE_SIZE` 件だけを同期し、上端付近で `loadOlder()` が古い方へ 1 ページずつ広げる。
  const messagesQuery = useMessageFeed(channelId);
  const feed = messagesQuery.data ?? [];
  const channelUpload = useAttachmentUpload(channelId);
  const e2ee = useE2ee();
  const { downgraded, checkingEncryption, treatAsEncrypted } = useEncryptedDisplay({
    channelId,
    channelName,
    encrypted,
    hasCiphertext: feed.some((row) => row.ciphertext != null),
  });
  // 暗号化チャンネルには、送る直前に Megolm で暗号化して送る（docs/e2ee.md）。このタブが暗号ストアを
  // 開いていない（別のタブが開いている、登録中、使えないブラウザー）あいだは、平文を送らないよう
  // 投稿、返信、編集を止める。サーバーは平文を拒否するが、拒否の前に本文が届いてしまうため。
  const [recipients, setRecipients] = useState<RecipientSummary | null>(null);
  const cryptoDeps = useMemo<ChannelCryptoDeps | null>(
    () =>
      encrypted && e2ee.worker && e2ee.deviceId
        ? {
            worker: e2ee.worker,
            client: appClient,
            deviceId: e2ee.deviceId,
            onRecipients: (_channelId, summary) => setRecipients(summary),
            onSessionCreated: () => e2ee.requestKeyBackup(),
          }
        : null,
    [appClient, e2ee, encrypted],
  );
  const sendBlocked = (encrypted && cryptoDeps === null) || downgraded || checkingEncryption;
  // 暗号化チャンネルの添付は、暗号化して上げ、鍵などを Megolm で暗号化した本文の attachments に入れる
  // （#364。docs/e2ee.md の「暗号化した添付」）。Composer には今までどおり ID を返し、本文に入れる
  // AttachmentInfo は送信まで ID ごとに持っておく。送れないあいだ（sendBlocked。cryptoDeps がない、格下げ、確認待ち）は出さない。
  const encryptedUpload = useEncryptedAttachmentUpload(channelId);
  const encryptedAttachments = useRef(new Map<string, AttachmentInfo>());
  const uploadEncryptedAttachment = useCallback(
    async (file: File): Promise<UploadedAttachment> => {
      const info = await encryptedUpload(file);
      encryptedAttachments.current.set(info.attachment_id, info);
      return {
        id: info.attachment_id,
        file_name: info.name,
        file_size: info.size,
        content_type: info.mimetype,
      };
    },
    [encryptedUpload],
  );
  const takeEncryptedAttachments = useCallback(
    (attachmentIds: string[], sensitiveAttachmentIds: string[]): AttachmentInfo[] =>
      attachmentIds.map((id) => {
        const info = encryptedAttachments.current.get(id);
        if (!info) throw new Error(`Unknown encrypted attachment: ${id}`);
        return sensitiveAttachmentIds.includes(id) ? { ...info, sensitive: true } : info;
      }),
    [],
  );
  const forgetEncryptedAttachments = useCallback((attachmentIds: string[]) => {
    for (const id of attachmentIds) encryptedAttachments.current.delete(id);
  }, []);
  const uploadAttachment = treatAsEncrypted
    ? cryptoDeps && !sendBlocked
      ? uploadEncryptedAttachment
      : undefined
    : channelUpload;
  const { currentUserId, isAdmin } = useCurrentUser();
  const { locale } = useLocale();
  const channelsQuery = useActiveChannels();
  const myMemberships = useMyChannelMemberships();
  const commandSuggestions = useChannelCommands(channelId, currentUserId);
  const { isOnline } = useOfflineStatus();
  const typing = useTyping(channelId, null, !readOnly && isMember && !!currentUserId);
  const { error, errorCause, setError, run, isPending: isWritePending } = useWriteRunner();

  const { threadRootId, openThread, closeThread } = useShell();
  const threadQuery = useThreadFeed(channelId, threadRootId);
  const threadReplies = threadQuery.data ?? [];
  const threadTyping = useTyping(
    channelId,
    threadRootId,
    !readOnly && isMember && !!currentUserId && !!threadRootId,
  );
  const [legacyThreadTarget, setLegacyThreadTarget] = useState<{ root: string; target: string }>();
  const threadMembership = useThreadMembership(currentUserId, threadRootId);
  const retryMembership = threadMembership.retry;
  // 参加が必要な 403 への対応（平文・暗号化の返信で共通）。表示している参加状態が古かった（ほかの画面・
  // クライアントで退出した、など）ので、参加状態を取り直す。すると入力欄の代わりに参加ボタンが出る。
  // 入力欄は隠れるだけなので、下書きは残る。対象のエラーなら true を返す。
  const noticeJoinRequired = useCallback(
    (cause: unknown): boolean => {
      if (!isThreadJoinRequired(cause)) return false;
      void retryMembership?.();
      toast.info(t("threads.joinToReply"), { id: "thread-join-required" });
      return true;
    },
    [retryMembership, t],
  );
  const threadRootQuery = useThreadRoot(channelId, threadRootId);
  const threadRootRow = threadRootQuery.data?.[0] ?? null;
  const threadRoot = threadRootRow && !threadRootRow.deleted_at ? threadRootRow : null;
  // 暗号化チャンネルで、読み込んだうちで最初の暗号化したメッセージの作成日時（#367）。スレッドのペインで、
  // それより後の平文の行（正しいサーバーは作れない）に警告を付けるのに使う。チャンネルのルートに暗号化した
  // 行がなくても、開いているスレッドの暗号化した返信より後の平文に気付けるよう、スレッドの行も含める。
  const encryptedSince = useMemo(
    () => firstEncryptedAt([...feed, ...(threadRoot ? [threadRoot] : []), ...threadReplies]),
    [feed, threadRoot, threadReplies],
  );
  const threadIsArchived = !!threadRoot?.thread_archive_at;
  // Legacy links to nested threads resolve to the canonical root. Message IDs
  // themselves remain stable, so navigation can still highlight the old target.
  useEffect(() => {
    const canonical = threadMembership.data?.id;
    if (canonical && canonical !== threadRootId) {
      setLegacyThreadTarget({ root: canonical, target: threadRootId! });
      openThread(canonical);
    }
  }, [threadMembership.data?.id, threadRootId, openThread]);
  // 移行中の子スレッドへ送った返信は最上位へ正規化される。確定した親へ表示を移す。
  useEffect(() => {
    const onRedirect = (event: Event) => {
      const { from, to } = (event as CustomEvent<ThreadReplyRedirect>).detail;
      if (from === threadRootId) openThread(to);
    };
    window.addEventListener(THREAD_REPLY_REDIRECTED, onRedirect);
    return () => window.removeEventListener(THREAD_REPLY_REDIRECTED, onRedirect);
  }, [threadRootId, openThread]);
  useEffect(() => {
    if (
      legacyThreadTarget &&
      threadRootId !== legacyThreadTarget.root &&
      threadRootId !== legacyThreadTarget.target
    ) {
      setLegacyThreadTarget(undefined);
    }
  }, [threadRootId, legacyThreadTarget]);
  const decryptionRows = useMemo(
    () =>
      treatAsEncrypted
        ? [
            ...(messagesQuery.data ?? []),
            ...(threadQuery.data ?? []),
            ...(threadRoot ? [threadRoot] : []),
          ]
        : [],
    [treatAsEncrypted, messagesQuery.data, threadQuery.data, threadRoot],
  );
  const workerDecryptions = useMessageDecryptions(channelId, decryptionRows);
  // 暗号化チャンネルの、暗号文がなく `epoch` だけがある（削除されていない）行は、暗号ストアがなくても本文を
  // 出さない。正しいサーバーは DB のトリガーでこの行を作れないので、同期元が書き換えたものとして扱う。
  // `epoch` もない行は、暗号化に切り替える前の平文のメッセージ（#367）なので、本文を出す（MessageRow）。
  // 区切りより上の行には印を付けず、暗号化したメッセージより後ろにある行だけ警告を付ける（チャンネルの一覧は
  // MessageList が並びから、スレッドのペインは `encryptedSince` の日時から決める）。
  const decryptions = useMemo(
    () => withUnencryptedRows(decryptionRows, workerDecryptions),
    [decryptionRows, workerDecryptions],
  );

  /** 暗号化した送信の失敗を利用者に知らせる。 */
  const reportEncryptedFailure = useCallback(
    (cause: unknown) => {
      if (noticeJoinRequired(cause)) return;
      const code = (cause as { code?: unknown } | null)?.code;
      if (code === "NeedsUnlock") toast.error(t("e2ee.sendNeedsUnlock"));
      else if (cause instanceof EncryptedSendError) toast.error(t("e2ee.sendStaleEpoch"));
      else if (apiErrorCode(cause) === "E2EE_STALE_REVISION") toast.error(t("e2ee.editConflict"));
      else toast.error(t("common.writeErrorDetail", { error: localizeApiError(cause, t) }));
    },
    [noticeJoinRequired, t],
  );

  const attachmentsQuery = useMessageAttachments(
    channelId,
    [...feed, ...threadReplies, ...(threadRoot ? [threadRoot] : [])].some(
      (row) => !row.display_data,
    ),
  );
  const attachmentsByMessage = useMemo(
    () => groupAttachmentsByMessage(attachmentsQuery.data ?? []),
    [attachmentsQuery.data],
  );

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  // 暗号化チャンネルで編集を始めたときの版。保存はこの次の版で送り、ほかで編集されていれば 409 にする。
  const [editingRevision, setEditingRevision] = useState(0);
  const savingEdit = useRef(false);
  const editingIdRef = useRef(editingId);
  editingIdRef.current = editingId;
  const editingTextRef = useRef(editingText);
  editingTextRef.current = editingText;
  const decryptionsRef = useRef(decryptions);
  decryptionsRef.current = decryptions;
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [isJoining, setIsJoining] = useState(false);

  const isArchivedMessage = useCallback(
    (id: string) => {
      const message = messagesCollection.get(id);
      const rootId = message?.parent_id ?? id;
      return (
        !!messagesCollection.get(rootId)?.thread_archive_at ||
        (threadIsArchived &&
          (id === threadRootId ||
            message?.parent_id === threadRootId ||
            threadReplies.some((reply) => reply.id === id)))
      );
    },
    [messagesCollection, threadIsArchived, threadRootId, threadReplies],
  );

  // 親が削除されたスレッドの返信（検索・リンクで開いたまま読むだけにするもの）。
  const isUnderDeletedRoot = useCallback(
    (id: string) =>
      !!threadRootRow?.deleted_at &&
      (messagesCollection.get(id)?.parent_id === threadRootId ||
        threadReplies.some((reply) => reply.id === id)),
    [threadRootRow?.deleted_at, messagesCollection, threadRootId, threadReplies],
  );

  useEffect(() => {
    // 開いた後に親が削除されたら、その返信の編集・削除確認も閉じる。
    if (editingId && (isArchivedMessage(editingId) || isUnderDeletedRoot(editingId))) {
      setEditingId(null);
      setEditingText("");
    }
    if (
      pendingDeleteId &&
      (isArchivedMessage(pendingDeleteId) || isUnderDeletedRoot(pendingDeleteId))
    )
      setPendingDeleteId(null);
  }, [editingId, pendingDeleteId, isArchivedMessage, isUnderDeletedRoot]);

  // 編集中にチャンネルが暗号化に切り替わったら（#367）、切り替える前の平文のメッセージの編集をやめる。
  // サーバーも 409 `E2EE_LEGACY_IMMUTABLE` で拒否する。
  useEffect(() => {
    if (!treatAsEncrypted || !editingId) return;
    const row = messagesCollection.get(editingId);
    if (row && isPlaintextBeforeEncryption(row)) {
      setEditingId(null);
      setEditingText("");
      toast.info(t("e2ee.legacy.editCancelled"));
    }
  }, [treatAsEncrypted, editingId, messagesCollection, t]);

  const isBlockedReactionMessage = useCallback(
    (id: string) => isArchivedMessage(id) || isUnderDeletedRoot(id),
    [isArchivedMessage, isUnderDeletedRoot],
  );
  const channelReactions = useChannelReactions({
    channelId,
    currentUserId,
    users: usersQuery.data,
    usersLoading: usersQuery.isLoading,
    readOnly,
    isBlockedMessage: isBlockedReactionMessage,
    runWrite: run,
  });

  // 通知・リンクの対象 (`?message=`)。メイン一覧の範囲とは関係なく、その行だけを取る。
  const resolvedTargetId =
    targetMessageId ??
    (legacyThreadTarget?.root === threadRootId ? legacyThreadTarget.target : undefined);
  const targetQuery = useMessageById(channelId, resolvedTargetId ?? null);
  const target = targetQuery.data?.[0];

  // 対象が返信なら、そのスレッドを開く。
  useEffect(() => {
    if (target?.parent_id && !target.deleted_at) openThread(target.parent_id);
  }, [target?.id, target?.parent_id, target?.deleted_at, openThread]);

  // 対象がトップレベルで、いま読み込んでいる範囲より古いときは、含まれるまで 1 ページずつ古い方へ広げる
  // (連続した範囲になるので、間のメッセージが欠けない)。広げる回数には上限がある。
  const revealedPages = useRef({ id: "", pages: 0 });
  const { hasOlder, isLoadingOlder, loadOlder } = messagesQuery;
  useEffect(() => {
    if (!target || target.deleted_at || target.parent_id) return;
    if (revealedPages.current.id !== target.id) revealedPages.current = { id: target.id, pages: 0 };
    if (feed.some((row) => row.id === target.id)) return;
    if (!hasOlder || isLoadingOlder) return;
    if (revealedPages.current.pages >= MAX_REVEAL_PAGES) {
      toast.info(t("threads.targetTooOld"), { id: "target-too-old" });
      return;
    }
    revealedPages.current.pages += 1;
    void loadOlder();
  }, [target, feed, hasOlder, isLoadingOlder, loadOlder, t]);

  // 対象がスレッドの返信なら、スレッドの範囲も、含まれるまで広げる。
  const revealedThreadPages = useRef({ id: "", pages: 0 });
  const { hasOlder: threadHasOlder, isLoadingOlder: threadLoadingOlder } = threadQuery;
  const loadOlderReplies = threadQuery.loadOlder;
  useEffect(() => {
    if (!target || target.deleted_at || !target.parent_id || target.parent_id !== threadRootId) {
      return;
    }
    if (revealedThreadPages.current.id !== target.id) {
      revealedThreadPages.current = { id: target.id, pages: 0 };
    }
    if (threadReplies.some((row) => row.id === target.id)) return;
    if (!threadHasOlder || threadLoadingOlder) return;
    if (revealedThreadPages.current.pages >= MAX_REVEAL_PAGES) {
      toast.info(t("threads.targetTooOld"), { id: "target-too-old" });
      return;
    }
    revealedThreadPages.current.pages += 1;
    void loadOlderReplies();
  }, [
    target,
    threadRootId,
    threadReplies,
    threadHasOlder,
    threadLoadingOlder,
    loadOlderReplies,
    t,
  ]);

  // 検索・リンクで開いた返信の親が削除されていても、返信は残っている。そのスレッドは閉じずに、親の代わりに
  // 知らせを出して読むだけにする。
  const deletedRootForTarget =
    !!threadRootRow?.deleted_at &&
    !!target &&
    !target.deleted_at &&
    target.parent_id === threadRootId;
  const threadPaneRoot = threadRoot ?? (deletedRootForTarget ? threadRootRow : null);

  // スレッドの親が (他ユーザーの操作などで) 見えなくなったらスレッドも閉じる。
  // 論理削除なら、返信が残っていることを利用者に伝えてから閉じる。親は、メイン一覧の範囲とは関係なく、
  // ID で取る。取得が終わるまでは判断しない。
  useEffect(() => {
    if (!threadRootId || threadRootQuery.isLoading || threadRootQuery.isError) {
      return;
    }
    if (threadRootRow && !threadRootRow.deleted_at) return;
    if (deletedRootForTarget) return;
    // 検索・リンクの対象（返信）を取り終えるまでは決めない。先に閉じると対象の指定も消え、開き直せない。
    if (resolvedTargetId && targetQuery.isLoading) return;
    closeThread();
    if (threadRootRow?.deleted_at) {
      toast.info(t("threads.rootDeleted"), { id: "thread-root-deleted" });
    }
  }, [
    threadRootId,
    threadRootRow,
    threadRootQuery.isLoading,
    threadRootQuery.isError,
    deletedRootForTarget,
    resolvedTargetId,
    targetQuery.isLoading,
    closeThread,
    t,
  ]);

  // 書き込み失敗は sonner で通知する（既存の `useWriteRunner` の error を監視）。
  // `error` は英語の技術メッセージになり得るため、生の原因があれば localizeApiError で
  // 現在のロケールの文言に換えてから出す（暗号化送信の失敗と同じ経路）。
  useEffect(() => {
    if (!error) {
      return;
    }
    if (noticeJoinRequired(errorCause)) {
      setError(null);
      return;
    }
    const description = errorCause == null ? error : localizeApiError(errorCause, t);
    toast.error(t("common.writeError"), { description, id: "message-write-error" });
    setError(null);
  }, [error, errorCause, noticeJoinRequired, setError, t]);

  const handleJoin = useCallback(async () => {
    if (!currentUserId || isJoining) {
      return;
    }
    setIsJoining(true);
    try {
      await run(() =>
        channelUsers.insert(
          {
            user_id: currentUserId,
            channel_id: channelId,
            joined_at: nowIso(),
          },
          { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
        ),
      );
    } finally {
      setIsJoining(false);
    }
  }, [channelId, channelUsers, currentUserId, isJoining, run]);

  const runCommand = useCallback(
    async (content: string, appId?: string): Promise<boolean | null> => {
      const command = /^\/([a-zA-Z0-9_-]+)(?:\s+([\s\S]*))?$/.exec(content.trim());
      if (!command) return null;
      try {
        if (appId) {
          await appClient.request(`/api/apps/${encodeURIComponent(appId)}/invoke`, {
            method: "POST",
            body: {
              kind: "command",
              name: command[1],
              channel_id: channelId,
              values: { text: command[2] ?? "" },
            },
          });
        } else {
          await appClient.request(apiPath`/api/apps/commands/${command[1]}`, {
            method: "POST",
            body: { channel_id: channelId, text: command[2] ?? "" },
          });
        }
        refreshAppSurfaces();
        return true;
      } catch (error) {
        if (!appId && error instanceof ApiError && error.status === 404) return null;
        toast.error(t("apps.actionError"));
        return false;
      }
    },
    [appClient, channelId, t],
  );

  const handleSend = useCallback(
    async (
      content: string,
      commandAppId?: string,
      attachmentIds: string[] = [],
      sensitiveAttachmentIds: string[] = [],
    ): Promise<boolean> => {
      if (sendBlocked) return false;
      if (cryptoDeps) {
        // 暗号化チャンネルでは App のコマンドを使えない（本文を App に送らない）。本文として暗号化して送る。
        try {
          await sendEncryptedMessage(cryptoDeps, {
            channelId,
            shapeEpoch: membershipEpoch,
            messageId: crypto.randomUUID(),
            parentId: null,
            body: content,
            mentions: encryptedMentions(content, usersQuery.data ?? []),
            attachments: takeEncryptedAttachments(attachmentIds, sensitiveAttachmentIds),
          });
          forgetEncryptedAttachments(attachmentIds);
          return true;
        } catch (cause) {
          reportEncryptedFailure(cause);
          return false;
        }
      }
      if (attachmentIds.length === 0) {
        const commandResult = await runCommand(content, commandAppId);
        if (commandResult !== null) return commandResult;
      }
      const id = crypto.randomUUID();
      const metadata =
        attachmentIds.length > 0
          ? {
              attachment_ids: attachmentIds,
              ...(sensitiveAttachmentIds.length
                ? { sensitive_attachment_ids: sensitiveAttachmentIds }
                : {}),
            }
          : undefined;
      await run(() =>
        messagesCollection.insert(
          {
            id,
            user_id: currentUserId ?? "",
            channel_id: channelId,
            content,
            created_at: nowIso(),
            updated_at: null,
            deleted_at: null,
            parent_id: null,
          },
          metadata ? { metadata } : undefined,
        ),
      );
      // `run` は失敗を握ってロールバックするため、確定後に楽観行が残っているかで
      // 成否を判定する（Composer が失敗時に下書きを復元できるようにする）。
      return messagesCollection.get(id) !== undefined;
    },
    [
      run,
      messagesCollection,
      currentUserId,
      channelId,
      runCommand,
      sendBlocked,
      cryptoDeps,
      membershipEpoch,
      usersQuery.data,
      reportEncryptedFailure,
      takeEncryptedAttachments,
      forgetEncryptedAttachments,
    ],
  );

  const handleSendReply = useCallback(
    async (
      content: string,
      commandAppId?: string,
      attachmentIds: string[] = [],
      sensitiveAttachmentIds: string[] = [],
    ): Promise<boolean> => {
      if (sendBlocked) return false;
      if (readOnly || !isMember) {
        toast.info(t("messages.joinToPostDescription"), { id: "thread-reply-join-required" });
        return false;
      }
      if (cryptoDeps) {
        if (!threadRootId) return false;
        try {
          await sendEncryptedMessage(cryptoDeps, {
            channelId,
            shapeEpoch: membershipEpoch,
            messageId: crypto.randomUUID(),
            parentId: threadRootId,
            body: content,
            mentions: encryptedMentions(content, usersQuery.data ?? []),
            attachments: takeEncryptedAttachments(attachmentIds, sensitiveAttachmentIds),
          });
          window.dispatchEvent(new Event(THREADS_CHANGED));
          forgetEncryptedAttachments(attachmentIds);
          return true;
        } catch (cause) {
          reportEncryptedFailure(cause);
          return false;
        }
      }
      if (attachmentIds.length === 0) {
        const commandResult = await runCommand(content, commandAppId);
        if (commandResult !== null) return commandResult;
      }
      const rootId = threadRootId;
      if (!rootId) {
        return false;
      }
      const id = crypto.randomUUID();
      const metadata =
        attachmentIds.length > 0
          ? {
              attachment_ids: attachmentIds,
              ...(sensitiveAttachmentIds.length
                ? { sensitive_attachment_ids: sensitiveAttachmentIds }
                : {}),
            }
          : undefined;
      await run(() =>
        messagesCollection.insert(
          {
            id,
            user_id: currentUserId ?? "",
            channel_id: channelId,
            content,
            created_at: nowIso(),
            updated_at: null,
            deleted_at: null,
            parent_id: rootId,
          },
          metadata ? { metadata } : undefined,
        ),
      );
      return messagesCollection.get(id) !== undefined;
    },
    [
      run,
      messagesCollection,
      currentUserId,
      channelId,
      threadRootId,
      isMember,
      readOnly,
      t,
      runCommand,
      sendBlocked,
      cryptoDeps,
      membershipEpoch,
      usersQuery.data,
      reportEncryptedFailure,
      takeEncryptedAttachments,
      forgetEncryptedAttachments,
    ],
  );

  const handleStartEdit = useCallback(
    (id: string, content: string) => {
      // 暗号化チャンネルの編集も、暗号化して送れないあいだは始めない。
      if (sendBlocked || isUnderDeletedRoot(id)) return;
      if (encrypted) {
        // 行の `content` は空なので、復号した本文から始める。復号できない版は上書きしない。
        const decrypted = decryptions.get(id);
        // 版の照合で警告した行（表示と行の版が違う）からは編集を始めない。
        if (decrypted?.status !== "decrypted" || decrypted.revisionWarning) return;
        setEditingId(id);
        setEditingText(decrypted.body);
        // 表示している本文の版を基準にする。行が新しい版に進んでいて、その復号がまだなら、保存は
        // 409 になり、ほかで編集されたと知らせる（新しい版を古い本文で上書きしない）。
        setEditingRevision(decrypted.rowRevision ?? messagesCollection.get(id)?.revision ?? 0);
        return;
      }
      setEditingId(id);
      setEditingText(content);
    },
    [sendBlocked, encrypted, decryptions, messagesCollection, isUnderDeletedRoot],
  );

  const handleCancelEdit = useCallback(() => {
    setEditingId(null);
    setEditingText("");
  }, []);

  const handleSaveEdit = useCallback(async () => {
    const id = editingId;
    const content = editingText.trim();
    if (!id || isArchivedMessage(id) || isUnderDeletedRoot(id) || sendBlocked) {
      return;
    }
    const row =
      messagesCollection.get(id) ??
      feed.find((item) => item.id === id) ??
      (threadRoot?.id === id ? threadRoot : undefined) ??
      threadReplies.find((item) => item.id === id);
    const rowAttachments = row?.display_data?.attachments ?? attachmentsByMessage.get(id) ?? [];
    const allowEmpty =
      !cryptoDeps && (rowAttachments.length > 0 || (row?.forwarded_message_ids?.length ?? 0) > 0);
    if (!content && !allowEmpty) {
      return;
    }
    if (cryptoDeps) {
      const targetRow = messagesCollection.get(id);
      const decrypted = decryptions.get(id);
      if (!targetRow || decrypted?.status !== "decrypted" || savingEdit.current) return;
      // 送れるまで編集中の本文を残す（失敗したら、そのまま直して保存し直せる）。
      savingEdit.current = true;
      try {
        await editEncryptedMessage(cryptoDeps, {
          channelId,
          shapeEpoch: membershipEpoch,
          messageId: id,
          parentId: targetRow.parent_id,
          revision: editingRevision + 1,
          body: content,
          kind: decrypted.kind,
          mentions: encryptedMentions(content, usersQuery.data ?? []),
          // 編集では添付を変えられないので、元の版の添付をそのまま入れる（#364）。
          attachments: decrypted.attachments,
        });
        // 保存中に別のメッセージの編集を始めた、または本文を書き足したら、編集欄を閉じない。
        if (editingIdRef.current === id && editingTextRef.current.trim() === content) {
          setEditingId(null);
          setEditingText("");
        } else if (editingIdRef.current === id) {
          // 書き足した本文を保存し直すときは、今送った版の次の版で送る。
          setEditingRevision(editingRevision + 1);
        }
      } catch (cause) {
        // 応答をなくして保存し直したときは、今の版が自分の送った本文なら済んだものとして扱う。
        const shown = decryptionsRef.current.get(id);
        if (editAlreadyApplied(cause, shown, content)) {
          if (editingIdRef.current === id && editingTextRef.current.trim() === content) {
            setEditingId(null);
            setEditingText("");
          } else if (editingIdRef.current === id && shown?.status === "decrypted") {
            setEditingRevision(shown.rowRevision ?? editingRevision + 1);
          }
        } else {
          reportEncryptedFailure(cause);
        }
      } finally {
        savingEdit.current = false;
      }
      return;
    }
    setEditingId(null);
    setEditingText("");
    await run(() =>
      messagesCollection.update(id, (message) => {
        message.content = content;
      }),
    );
    forgetForwardIds(
      { apiBaseUrl: config?.apiBaseUrl ?? DEFAULT_CONFIG.apiBaseUrl, currentUserId },
      [id],
    );
  }, [
    editingId,
    editingText,
    editingRevision,
    run,
    messagesCollection,
    feed,
    threadRoot,
    threadReplies,
    attachmentsByMessage,
    isArchivedMessage,
    isUnderDeletedRoot,
    sendBlocked,
    cryptoDeps,
    decryptions,
    channelId,
    membershipEpoch,
    usersQuery.data,
    reportEncryptedFailure,
    config?.apiBaseUrl,
    currentUserId,
  ]);

  const handleConfirmDelete = useCallback(async () => {
    const id = pendingDeleteId;
    setPendingDeleteId(null);
    if (!id || isArchivedMessage(id) || isUnderDeletedRoot(id)) {
      return;
    }
    // 論理削除は `deleted_at` の update。Electric が流す operation と揃えて
    // delete→insert の再出現（ちらつき）を避ける（Phase 3 の扱いを踏襲）。
    await run(() =>
      messagesCollection.update(id, (message) => {
        message.deleted_at = nowIso();
      }),
    );
    forgetForwardIds(
      { apiBaseUrl: config?.apiBaseUrl ?? DEFAULT_CONFIG.apiBaseUrl, currentUserId },
      [id],
    );
  }, [
    pendingDeleteId,
    run,
    messagesCollection,
    isArchivedMessage,
    isUnderDeletedRoot,
    config?.apiBaseUrl,
    currentUserId,
  ]);

  const forwardedIds = useMemo(() => {
    if (treatAsEncrypted) return [];
    const rows = [...feed, ...threadReplies, ...(threadRoot ? [threadRoot] : [])];
    return rows.flatMap((row) => row.forwarded_message_ids ?? []);
  }, [treatAsEncrypted, feed, threadReplies, threadRoot]);
  const forwardedMessages = useForwardedMessages(forwardedIds, !treatAsEncrypted);

  const currentChannel = useMemo(
    () => (channelsQuery.data ?? []).find((channel) => channel.id === channelId) ?? null,
    [channelsQuery.data, channelId],
  );
  const forwardDestinations = useMemo(
    () =>
      buildForwardDestinations({
        channels: channelsQuery.data ?? [],
        memberships: myMemberships.allMemberships ?? myMemberships.memberships,
        users: usersQuery.data ?? [],
        currentUserId,
        isAdmin,
        locale,
        t,
      }),
    [
      channelsQuery.data,
      myMemberships.allMemberships,
      myMemberships.memberships,
      usersQuery.data,
      currentUserId,
      isAdmin,
      locale,
      t,
    ],
  );
  const [forwardingSource, setForwardingSource] = useState<ResolvedForwardMessage | null>(null);
  const [isForwarding, setIsForwarding] = useState(false);
  const [forwardError, setForwardError] = useState<string | null>(null);
  // 再試行時はサーバー側の冪等性（同じ client UUID）を効かせ、宛先やコメントを変えた場合のみ新規採番する
  const pendingForwardKeyRef = useRef<{
    sourceId: string;
    destinationChannelId: string;
    comment: string;
    id: string;
  } | null>(null);

  const handleOpenForward = useCallback(
    (msg: FeedMessage, msgAttachments: AttachmentRow[]) => {
      if (treatAsEncrypted || !currentUserId) return;
      const participantUsers = (members ?? []).flatMap((member) => {
        const user = (usersQuery.data ?? []).find((candidate) => candidate.id === member.user_id);
        return user
          ? [{ id: user.id, user_name: user.user_name, display_name: user.display_name }]
          : [];
      });
      pendingForwardKeyRef.current = null;
      setForwardError(null);
      setForwardingSource({
        id: msg.id,
        channel_id: msg.channel_id ?? channelId,
        channel_name: currentChannel?.channel_name ?? channelName ?? "",
        is_private: currentChannel?.is_private ?? false,
        is_dm: currentChannel?.is_dm ?? false,
        dm_participants: participantUsers,
        parent_id: msg.parent_id,
        user_id: msg.user_id,
        created_at: msg.created_at,
        updated_at: msg.updated_at ?? null,
        content: msg.content,
        card: msg.card ?? null,
        components: msg.components ?? null,
        message_kind: msg.message_kind ?? null,
        author: {
          id: msg.user_id,
          user_name: msg.user_name ?? msg.user_id,
          display_name: msg.display_name ?? msg.user_name ?? msg.user_id,
          avatar_url: msg.avatar_url ?? null,
          is_bot: !!msg.is_bot,
          app_id: msg.app_id ?? null,
        },
        emojis: msg.display_data?.emojis ?? {},
        attachments: msgAttachments,
        has_forwards: (msg.forwarded_message_ids?.length ?? 0) > 0,
      });
    },
    [
      treatAsEncrypted,
      currentUserId,
      members,
      usersQuery.data,
      channelId,
      currentChannel,
      channelName,
    ],
  );

  const handleSubmitForward = useCallback(
    async (destinationChannelId: string, comment: string) => {
      if (!forwardingSource || isForwarding) return;
      setIsForwarding(true);
      setForwardError(null);
      try {
        const existingKey = pendingForwardKeyRef.current;
        const messageId =
          existingKey &&
          existingKey.sourceId === forwardingSource.id &&
          existingKey.destinationChannelId === destinationChannelId &&
          existingKey.comment === comment
            ? existingKey.id
            : crypto.randomUUID();
        pendingForwardKeyRef.current = {
          sourceId: forwardingSource.id,
          destinationChannelId,
          comment,
          id: messageId,
        };
        const body: CreateMessageRequest = {
          id: messageId,
          channel_id: destinationChannelId,
          content: comment,
          forwarded_message_ids: [forwardingSource.id],
        };
        await appClient.post<MessageResponse>("/api/messages", body);
        const targetDest = forwardDestinations.find((dest) => dest.id === destinationChannelId);
        pendingForwardKeyRef.current = null;
        setForwardingSource(null);
        toast.success(
          targetDest
            ? t("messages.forwardedTo", {
                channel: targetDest.isDm ? targetDest.label : `#${targetDest.label}`,
              })
            : t("messages.forwarded"),
        );
      } catch (cause) {
        if (cause instanceof ApiError && cause.status === 404) {
          setForwardError(t("messages.forwardSourceUnavailable"));
        } else {
          setForwardError(localizeApiError(cause, t));
        }
      } finally {
        setIsForwarding(false);
      }
    },
    [forwardingSource, isForwarding, appClient, forwardDestinations, t],
  );

  const threadTargetMessageId =
    resolvedTargetId &&
    threadPaneRoot &&
    resolvedTargetId !== threadPaneRoot.id &&
    (threadReplies.some((reply) => reply.id === resolvedTargetId) ||
      (target?.id === resolvedTargetId && target.parent_id === threadPaneRoot.id))
      ? resolvedTargetId
      : undefined;

  return (
    <MessageDecryptionsProvider
      decryptions={decryptions}
      mentionUserNames={mentionUserNames}
      encrypted={treatAsEncrypted}
      encryptedSince={encryptedSince}
    >
      <ForwardedMessagesProvider value={forwardedMessages}>
        <section className="relative flex h-full min-h-0">
          <div className="flex min-w-0 flex-1 flex-col">
            {channelReactions.isError ||
            messagesQuery.isError ||
            messagesQuery.persistedStatus === "error" ? (
              <div
                role="status"
                className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b px-4 py-2"
              >
                {channelReactions.isError ? (
                  <span className="text-xs text-destructive">{t("reactions.fetchError")}</span>
                ) : null}
                {messagesQuery.isError ? (
                  <span
                    className={
                      feed.length > 0 ? "text-xs text-muted-foreground" : "text-xs text-destructive"
                    }
                  >
                    {feed.length > 0 ? t("offline.cached") : t("messages.fetchError")}
                  </span>
                ) : null}
                {messagesQuery.persistedStatus === "error" ? (
                  <span className="text-xs text-destructive">
                    {t("offline.persistenceErrorTitle")}
                  </span>
                ) : null}
              </div>
            ) : null}

            <MessageList
              targetMessageId={targetMessageId}
              key={`${channelId}:${currentUserId}`}
              channelId={channelId}
              messages={feed}
              isWritePending={isWritePending}
              isLoading={messagesQuery.isLoading}
              isError={messagesQuery.isError}
              currentUserId={readOnly ? null : currentUserId}
              viewerId={isMember ? currentUserId : null}
              canModerate={canModerate}
              editingId={editingId}
              editingText={editingText}
              onEditingTextChange={setEditingText}
              onStartEdit={handleStartEdit}
              onCancelEdit={handleCancelEdit}
              onSaveEdit={handleSaveEdit}
              onRequestDelete={setPendingDeleteId}
              onForward={!treatAsEncrypted && currentUserId ? handleOpenForward : undefined}
              reactions={channelReactions.reactionGroups}
              attachments={attachmentsByMessage}
              kitchenCatalog={channelReactions.kitchenCatalog}
              canReact={channelReactions.canReact}
              onReaction={channelReactions.handleReaction}
              onPickReaction={channelReactions.openReactionPicker}
              onOpenThread={openThread}
              canCompose={isMember}
              isOffline={!isOnline}
              hasOlder={messagesQuery.hasOlder}
              isLoadingOlder={messagesQuery.isLoadingOlder}
              onLoadOlder={() => void messagesQuery.loadOlder()}
              onRetry={() => window.location.reload()}
            />

            {readOnly ? null : isMember ? (
              <>
                <TypingIndicator users={typing.users} />
                {downgraded ? (
                  <p role="alert" className="mx-3 mb-2 text-xs text-destructive">
                    {t("e2ee.downgraded")}
                  </p>
                ) : null}
                {/* 配布先の表示は、増えた人か配れない人がいるときだけ出る。中身が後から入っても読み上げられる
                  よう、live region は常に置いておく。 */}
                <div role="status" aria-live="polite" data-testid="encryption-recipients-status">
                  {encrypted && recipients ? (
                    <EncryptionRecipientsNotice
                      summary={recipients}
                      users={usersQuery.data ?? []}
                    />
                  ) : null}
                </div>
                <Composer
                  commands={commandSuggestions.commands}
                  onRequestCommands={commandSuggestions.load}
                  mentionUsers={mentionUsers}
                  mentionMemberIds={mentionMemberIds}
                  key={channelId}
                  onSend={handleSend}
                  onUploadAttachment={uploadAttachment}
                  onTyping={typing.onTyping}
                  // 最初のページが届く前の書き込みは、確認 (txid) を待つ間、永続化層が
                  // 最初のページの取得を止めるため、互いに待ち合って失敗する。
                  disabled={
                    readOnly ||
                    postingForbidden ||
                    sendBlocked ||
                    !isOnline ||
                    messagesQuery.isLoading
                  }
                  placeholder={
                    postingForbidden
                      ? t("channels.postingManagersOnly")
                      : downgraded
                        ? t("e2ee.downgraded")
                        : checkingEncryption
                          ? t("e2ee.checking")
                          : sendBlocked
                            ? t("e2ee.deviceNotReady")
                            : isOnline
                              ? undefined
                              : t("offline.composeDisabled")
                  }
                />
              </>
            ) : (
              <JoinToPostPrompt
                onJoin={() => void handleJoin()}
                joining={isJoining}
                disabled={!currentUserId || !isOnline}
              />
            )}
          </div>

          {threadPaneRoot ? (
            <ThreadPane
              commands={commandSuggestions.commands}
              onRequestCommands={commandSuggestions.load}
              targetMessageId={threadTargetMessageId}
              mentionUsers={mentionUsers}
              mentionMemberIds={mentionMemberIds}
              key={threadPaneRoot.id}
              readOnly={readOnly || !threadRoot}
              rootDeleted={!threadRoot}
              isArchived={threadIsArchived}
              isMember={!readOnly && isMember}
              isOffline={!isOnline}
              typingUsers={threadTyping.users}
              onTyping={threadTyping.onTyping}
              root={threadPaneRoot}
              replies={threadReplies}
              membership={threadMembership.data?.membership}
              membershipPending={threadMembership.pending}
              membershipError={threadMembership.error}
              membershipLoading={!threadMembership.data}
              participantCount={threadMembership.data?.participant_count}
              participants={threadMembership.data?.participants}
              onMembershipRetry={() => void threadMembership.retry?.()}
              onJoinThread={() => void threadMembership.change?.("join")}
              onLeaveThread={() => void threadMembership.change?.("leave")}
              onReopenThread={() => void threadMembership.change?.("unarchive")}
              onReopenAndJoinThread={() =>
                void (async () => {
                  // 再Openに失敗したら参加しない（アーカイブ中の参加は 409 になり、原因が分からなくなる）。
                  if (await threadMembership.change?.("unarchive")) {
                    await threadMembership.change?.("join");
                  }
                })()
              }
              isLoading={threadQuery.isLoading}
              isError={threadQuery.isError}
              currentUserId={readOnly || threadIsArchived || !threadRoot ? null : currentUserId}
              viewerId={currentUserId}
              mentionUserId={isMember ? currentUserId : null}
              canModerate={canModerate && !threadIsArchived && !!threadRoot}
              reactions={channelReactions.reactionGroups}
              attachments={attachmentsByMessage}
              onUploadAttachment={uploadAttachment}
              kitchenCatalog={channelReactions.kitchenCatalog}
              canReact={channelReactions.canReact && !threadIsArchived && !!threadRoot}
              onReaction={channelReactions.handleReaction}
              onPickReaction={channelReactions.openReactionPicker}
              editingId={editingId}
              editingText={editingText}
              onEditingTextChange={setEditingText}
              onStartEdit={handleStartEdit}
              onCancelEdit={handleCancelEdit}
              onSaveEdit={handleSaveEdit}
              onRequestDelete={setPendingDeleteId}
              onSendReply={handleSendReply}
              onOpenThread={openThread}
              onForward={!treatAsEncrypted && currentUserId ? handleOpenForward : undefined}
              composeDisabledReason={
                postingForbidden
                  ? t("channels.postingManagersOnly")
                  : downgraded
                    ? t("e2ee.downgraded")
                    : checkingEncryption
                      ? t("e2ee.checking")
                      : sendBlocked
                        ? t("e2ee.deviceNotReady")
                        : undefined
              }
              postingForbidden={postingForbidden}
              hasOlder={threadQuery.hasOlder}
              isLoadingOlder={threadQuery.isLoadingOlder}
              onLoadOlder={() => void threadQuery.loadOlder()}
              onRetry={() => window.location.reload()}
              onClose={closeThread}
            />
          ) : threadRootId && (threadRootQuery.isLoading || threadRootQuery.isError) ? (
            <ThreadPaneStatus
              isError={!!threadRootQuery.isError}
              onRetry={() => window.location.reload()}
              onClose={closeThread}
            />
          ) : null}

          <ReactionPicker
            open={channelReactions.reactionMessageId !== null}
            onOpenChange={(open) => {
              if (!open) {
                channelReactions.closeReactionPicker();
              }
            }}
            onSelect={(emoji) => {
              if (channelReactions.reactionMessageId) {
                void channelReactions.handleReaction(channelReactions.reactionMessageId, emoji);
              }
            }}
          />

          <ConfirmActionDialog
            open={pendingDeleteId !== null}
            onOpenChange={(open) => {
              if (!open) setPendingDeleteId(null);
            }}
            title={t("messages.deleteTitle")}
            description={t("messages.deleteDescription")}
            confirmLabel={t("messages.delete")}
            onConfirm={handleConfirmDelete}
          />

          <ForwardMessageDialog
            open={forwardingSource !== null}
            onOpenChange={(open) => {
              if (!open && !isForwarding) {
                pendingForwardKeyRef.current = null;
                setForwardingSource(null);
                setForwardError(null);
              }
            }}
            source={forwardingSource}
            destinations={forwardDestinations}
            isSubmitting={isForwarding}
            error={forwardError}
            onSubmit={handleSubmitForward}
          />
        </section>
      </ForwardedMessagesProvider>
    </MessageDecryptionsProvider>
  );
}
