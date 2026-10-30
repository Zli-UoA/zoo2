import { ChannelWebhooksDialog } from "./ChannelWebhooksDialog";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ArrowLeftIcon,
  HashIcon,
  HeadphonesIcon,
  LockIcon,
  LogOutIcon,
  MessagesSquareIcon,
  MoreHorizontalIcon,
  PencilIcon,
  ShieldAlertIcon,
  TrashIcon,
  UsersIcon,
} from "lucide-react";
import { useState } from "react";

import { useChannelAdmin } from "../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useMessageFeed } from "../../hooks/useRealtime";
import { useCall } from "../../lib/call/call-context";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelRow } from "../../lib/electric/types";
import type { ChannelUserWriteMetadata } from "../../lib/electric/writes";
import { localizeApiError } from "../../lib/i18n/apiError";
import { normalizeUtcTimestamp } from "../../lib/timestamps";
import { useShell } from "../layout/shell-context";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button, buttonVariants } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { ScrollArea } from "../ui/scroll-area";
import { Skeleton } from "../ui/skeleton";
import { normalizeChannelDescription } from "./channelDescription";
import { RenameChannelDialog } from "./RenameChannelDialog";
import { isGroupDm } from "./dm";
import { authorLabel, formatTime, fullTimestamp, type FeedMessage } from "./messages/messageFormat";

/** チャンネルヘッダ。名前・説明・メンバー数と管理メニュー、狭幅の戻る。 */
export function ChannelHeader({
  channel,
  memberCount,
  title,
  subtitle,
}: {
  channel: ChannelRow;
  /** 数値=人数、`null`=読み込み中、`undefined`=非表示（非公開で未参加）。 */
  memberCount?: number | null;
  title?: string;
  subtitle?: string;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const admin = useChannelAdmin(channel);
  const isDm = channel.is_dm;
  const isGroup = isDm && isGroupDm(channel.channel_name);
  const isArchived = admin.access.isArchived;
  const canManage = admin.access.canManageChannel && !isDm;
  const description = normalizeChannelDescription(channel.description);
  const displayTitle = title ?? channel.channel_name;
  const displaySubtitle = subtitle ?? (description || t("channels.noDescription"));
  const [confirmAction, setConfirmAction] = useState<"archive" | "unarchive" | null>(null);
  const [webhooksOpen, setWebhooksOpen] = useState(false);
  const [renameOpen, setRenameOpen] = useState(false);
  const [messageAdminOpen, setMessageAdminOpen] = useState(false);

  const call = useCall();
  const isInThisCall = call.currentChannelId === channel.id;
  const activeCallCount = call.activeCalls.get(channel.id) ?? 0;
  const hasActiveCall = activeCallCount > 0;

  async function handleCallClick() {
    if (isInThisCall) {
      await call.leaveCall();
    } else {
      await call.joinCall(channel.id);
    }
  }

  async function confirmArchive() {
    if (!confirmAction) {
      return;
    }
    const succeeded = await admin.setArchived(confirmAction === "archive");
    if (succeeded) {
      setConfirmAction(null);
    }
  }

  return (
    <header className="flex items-center gap-3 border-b bg-background/80 px-4 py-2.5 backdrop-blur supports-backdrop-filter:bg-background/60">
      <Button
        variant="ghost"
        size="icon-sm"
        className="md:hidden"
        aria-label={t("channels.back")}
        onClick={shell.showSidebar}
      >
        <ArrowLeftIcon />
      </Button>
      <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
        {isDm ? (
          isGroup ? (
            <UsersIcon className="size-4" aria-hidden="true" />
          ) : (
            <MessagesSquareIcon className="size-4" aria-hidden="true" />
          )
        ) : channel.is_private ? (
          <LockIcon className="size-4" aria-hidden="true" />
        ) : (
          <HashIcon className="size-4" aria-hidden="true" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <h2 className="truncate text-sm font-semibold">{displayTitle}</h2>
          {isArchived && !isDm ? (
            <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground">
              {t("moderation.archived")}
            </span>
          ) : null}
        </div>
        <p className="truncate text-xs text-muted-foreground">{displaySubtitle}</p>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {!isArchived ? (
          <Button
            variant={isInThisCall ? "secondary" : hasActiveCall ? "outline" : "ghost"}
            size="sm"
            onClick={() => void handleCallClick()}
            disabled={call.connectionState === "connecting"}
            aria-label={
              isInThisCall
                ? t("call.inCall")
                : hasActiveCall
                  ? `${t("call.join")} (${activeCallCount})`
                  : t("call.start")
            }
            title={
              isInThisCall
                ? t("call.inCall")
                : hasActiveCall
                  ? `${t("call.join")} (${activeCallCount})`
                  : t("call.start")
            }
            className={
              isInThisCall
                ? "bg-emerald-500/15 text-emerald-600 hover:bg-emerald-500/25 dark:text-emerald-400"
                : hasActiveCall
                  ? "border-emerald-500/40 text-emerald-600 hover:bg-emerald-500/10 dark:text-emerald-400"
                  : ""
            }
          >
            <HeadphonesIcon aria-hidden="true" />
            <span className="hidden sm:inline">
              {isInThisCall ? t("call.inCall") : hasActiveCall ? t("call.join") : t("call.start")}
            </span>
            {hasActiveCall && !isInThisCall ? (
              <span className="tabular-nums font-semibold text-emerald-600 dark:text-emerald-400">
                {activeCallCount}
              </span>
            ) : null}
          </Button>
        ) : null}
        {!isDm ? (
          <Button
            variant="outline"
            size="sm"
            onClick={shell.toggleMembers}
            aria-expanded={shell.membersOpen}
            aria-label={
              memberCount === undefined || memberCount === null
                ? t("members.title")
                : `${t("members.title")} ${memberCount}`
            }
          >
            <UsersIcon aria-hidden="true" />
            <span className="hidden sm:inline">{t("members.title")}</span>
            {memberCount === undefined ? null : (
              <span className="tabular-nums text-muted-foreground">
                {memberCount === null ? "…" : memberCount}
              </span>
            )}
          </Button>
        ) : null}
        {isDm ? <DmHeaderMenu channel={channel} /> : null}
        {canManage ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label={t("moderation.manageChannel")}
              title={t("moderation.manageChannel")}
              className={buttonVariants({ variant: "ghost", size: "icon-sm" })}
            >
              <MoreHorizontalIcon />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => setWebhooksOpen(true)}>
                {t("webhooks.title")}
              </DropdownMenuItem>
              {!isArchived ? (
                <DropdownMenuItem onClick={() => setRenameOpen(true)}>
                  <PencilIcon />
                  {t("channels.editTitle")}
                </DropdownMenuItem>
              ) : null}
              {isArchived ? (
                <DropdownMenuItem onClick={() => setConfirmAction("unarchive")}>
                  <ArchiveRestoreIcon />
                  {t("moderation.unarchive")}
                </DropdownMenuItem>
              ) : (
                <DropdownMenuItem onClick={() => setConfirmAction("archive")}>
                  <ArchiveIcon />
                  {t("moderation.archive")}
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={() => setMessageAdminOpen(true)}>
                <ShieldAlertIcon />
                {t("moderation.messageAdmin")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>

      {webhooksOpen && canManage ? (
        <ChannelWebhooksDialog key={channel.id} channel={channel} onOpenChange={setWebhooksOpen} />
      ) : null}

      {renameOpen && canManage && !isArchived ? (
        <RenameChannelDialog key={channel.id} channel={channel} onOpenChange={setRenameOpen} />
      ) : null}

      <AlertDialog
        open={!isDm && confirmAction !== null}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmAction(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirmAction === "archive"
                ? t("moderation.archiveTitle")
                : t("moderation.unarchiveTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirmAction === "archive"
                ? t("moderation.archiveDescription")
                : t("moderation.unarchiveDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancelShort")}</AlertDialogCancel>
            <AlertDialogAction
              variant={confirmAction === "archive" ? "destructive" : "default"}
              disabled={admin.pending !== null}
              onClick={() => void confirmArchive()}
            >
              {admin.pending === "archive" || admin.pending === "unarchive"
                ? t("moderation.executing")
                : t("moderation.execute")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {messageAdminOpen && canManage ? (
        <MessageAdminDialog
          channel={channel}
          open={messageAdminOpen}
          onOpenChange={setMessageAdminOpen}
          pending={admin.pending === "delete-message"}
          onDelete={admin.deleteMessage}
        />
      ) : null}
    </header>
  );
}

function DmHeaderMenu({ channel }: { channel: ChannelRow }) {
  const { t } = useTranslation();
  const shell = useShell();
  const navigate = useNavigate();
  const { channelUsers } = useCollections();
  const { currentUserId } = useCurrentUser();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function handleOpenChange(open: boolean) {
    if (!open && !pending) {
      setConfirmOpen(false);
      setError(null);
    }
  }

  async function leave() {
    if (!currentUserId || pending) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const transaction = channelUsers.delete(`${currentUserId}:${channel.id}`, {
        metadata: { action: "leave" } satisfies ChannelUserWriteMetadata,
      });
      await transaction.when("settled");
      setConfirmOpen(false);
      shell.showSidebar();
      await navigate({ to: "/" });
    } catch (cause) {
      setError(localizeApiError(cause, t));
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={t("dms.menu")}
          title={t("dms.menu")}
          className={cn(buttonVariants({ variant: "ghost", size: "icon-sm" }), "size-11 md:size-7")}
        >
          <MoreHorizontalIcon />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem
            variant="destructive"
            className="min-h-11"
            disabled={!currentUserId}
            onClick={() => {
              setError(null);
              setConfirmOpen(true);
            }}
          >
            <LogOutIcon />
            {t("dms.leave")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <AlertDialog open={confirmOpen} onOpenChange={handleOpenChange}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("dms.leaveTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("dms.leaveDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {error ? (
            <p className="text-xs text-destructive">{t("common.writeErrorDetail", { error })}</p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel className="min-h-11" disabled={pending}>
              {t("common.cancelShort")}
            </AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              className="min-h-11"
              disabled={pending || !currentUserId}
              onClick={() => void leave()}
            >
              {pending ? t("dms.leaving") : t("dms.leave")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function MessageAdminDialog({
  channel,
  open,
  onOpenChange,
  pending,
  onDelete,
}: {
  channel: ChannelRow;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pending: boolean;
  onDelete: (messageId: string) => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const messagesQuery = useMessageFeed(channel.id);
  const messages = messagesQuery.data ?? [];
  const [target, setTarget] = useState<FeedMessage | null>(null);

  async function confirmDelete() {
    if (!target) {
      return;
    }
    const succeeded = await onDelete(target.id);
    if (succeeded) {
      setTarget(null);
    }
  }

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="flex max-h-[85dvh] flex-col gap-0 overflow-hidden p-0 sm:max-w-lg">
          <DialogHeader className="border-b px-4 py-3">
            <DialogTitle>{t("moderation.messageAdmin")}</DialogTitle>
            <DialogDescription>{t("moderation.messageAdminDescription")}</DialogDescription>
          </DialogHeader>
          <ScrollArea className="min-h-0 flex-1">
            <ul className="flex flex-col gap-1 p-2">
              {messagesQuery.isLoading ? (
                [0, 1, 2].map((row) => <Skeleton key={row} className="h-16 w-full rounded-lg" />)
              ) : messagesQuery.isError ? (
                <li className="px-2 py-3 text-xs text-destructive">{t("messages.fetchError")}</li>
              ) : messages.length === 0 ? (
                <li className="px-2 py-3 text-xs text-muted-foreground">
                  {t("moderation.noMessages")}
                </li>
              ) : (
                messages.map((message) => (
                  <li
                    key={message.id}
                    className="flex items-start gap-3 rounded-lg px-2 py-2 hover:bg-accent/60"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-baseline gap-2">
                        <span className="truncate text-sm font-medium">{authorLabel(message)}</span>
                        <time
                          className="shrink-0 text-[11px] tabular-nums text-muted-foreground"
                          dateTime={normalizeUtcTimestamp(message.created_at)}
                          title={fullTimestamp(message.created_at)}
                        >
                          {formatTime(message.created_at)}
                        </time>
                      </div>
                      <p className="line-clamp-2 text-xs break-words whitespace-pre-wrap text-muted-foreground">
                        {message.content}
                      </p>
                    </div>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={t("moderation.deleteMessageLabel", {
                        author: authorLabel(message),
                      })}
                      title={t("moderation.forceDelete")}
                      className="mt-0.5 shrink-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
                      disabled={pending}
                      onClick={() => setTarget(message)}
                    >
                      <TrashIcon />
                    </Button>
                  </li>
                ))
              )}
            </ul>
          </ScrollArea>
        </DialogContent>
      </Dialog>

      <AlertDialog
        open={target !== null}
        onOpenChange={(next) => {
          if (!next) {
            setTarget(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("moderation.forceDeleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {target
                ? t("moderation.forceDeleteDescription", { author: authorLabel(target) })
                : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancelShort")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={pending}
              onClick={() => void confirmDelete()}
            >
              {pending ? t("moderation.deleting") : t("messages.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
