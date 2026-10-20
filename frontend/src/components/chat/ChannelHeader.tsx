import { useTranslation } from "react-i18next";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ArrowLeftIcon,
  HashIcon,
  LockIcon,
  MoreHorizontalIcon,
  ShieldAlertIcon,
  TrashIcon,
  UsersIcon,
} from "lucide-react";
import { useState } from "react";

import { useChannelAdmin } from "../../hooks/useChannelAdmin";
import { useMessageFeed } from "../../hooks/useRealtime";
import type { ChannelRow } from "../../lib/electric/types";
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
import { authorLabel, formatTime, fullTimestamp, type FeedMessage } from "./messages/messageFormat";

/** チャンネルヘッダ。名前・説明・メンバー数と管理メニュー、狭幅の戻る。 */
export function ChannelHeader({
  channel,
  memberCount,
}: {
  channel: ChannelRow;
  /** 数値=人数、`null`=読み込み中、`undefined`=非表示（非公開で未参加）。 */
  memberCount?: number | null;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const admin = useChannelAdmin(channel);
  const isArchived = admin.access.isArchived;
  const canManage = admin.access.canManageChannel;
  const [confirmAction, setConfirmAction] = useState<"archive" | "unarchive" | null>(null);
  const [messageAdminOpen, setMessageAdminOpen] = useState(false);

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
        {channel.is_private ? (
          <LockIcon className="size-4" aria-hidden="true" />
        ) : (
          <HashIcon className="size-4" aria-hidden="true" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <h2 className="truncate text-sm font-semibold">{channel.channel_name}</h2>
          {isArchived ? (
            <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground">
              アーカイブ済み
            </span>
          ) : null}
        </div>
        <p className="truncate text-xs text-muted-foreground">
          {channel.description ?? t("channels.noDescription")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <Button
          variant="outline"
          size="sm"
          onClick={shell.toggleMembers}
          aria-expanded={shell.membersOpen}
        >
          <UsersIcon />
          <span className="hidden sm:inline">{t("members.title")}</span>
          {memberCount === undefined ? null : (
            <span className="tabular-nums text-muted-foreground">
              {memberCount === null ? "…" : memberCount}
            </span>
          )}
        </Button>
        {canManage ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label="チャンネルの管理"
              title="チャンネルの管理"
              className={buttonVariants({ variant: "ghost", size: "icon-sm" })}
            >
              <MoreHorizontalIcon />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {isArchived ? (
                <DropdownMenuItem onClick={() => setConfirmAction("unarchive")}>
                  <ArchiveRestoreIcon />
                  アンアーカイブ
                </DropdownMenuItem>
              ) : (
                <DropdownMenuItem onClick={() => setConfirmAction("archive")}>
                  <ArchiveIcon />
                  アーカイブ
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={() => setMessageAdminOpen(true)}>
                <ShieldAlertIcon />
                メッセージ管理
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>

      <AlertDialog
        open={confirmAction !== null}
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
                ? "チャンネルをアーカイブしますか？"
                : "アーカイブを解除しますか？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirmAction === "archive"
                ? "アーカイブすると、メンバーはメッセージを閲覧できますが、新しい投稿や参加はできなくなります。"
                : "アーカイブを解除すると、メンバーは再び投稿できるようになります。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant={confirmAction === "archive" ? "destructive" : "default"}
              disabled={admin.pending !== null}
              onClick={() => void confirmArchive()}
            >
              {admin.pending === "archive" || admin.pending === "unarchive" ? "実行中…" : "実行"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {messageAdminOpen ? (
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
            <DialogTitle>メッセージ管理</DialogTitle>
            <DialogDescription>
              チャンネル内のメッセージを管理者として強制削除できます。
            </DialogDescription>
          </DialogHeader>
          <ScrollArea className="min-h-0 flex-1">
            <ul className="flex flex-col gap-1 p-2">
              {messagesQuery.isLoading ? (
                [0, 1, 2].map((row) => <Skeleton key={row} className="h-16 w-full rounded-lg" />)
              ) : messagesQuery.isError ? (
                <li className="px-2 py-3 text-xs text-destructive">
                  メッセージ取得に失敗しました。
                </li>
              ) : messages.length === 0 ? (
                <li className="px-2 py-3 text-xs text-muted-foreground">
                  メッセージがありません。
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
                          dateTime={message.created_at}
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
                      aria-label={`${authorLabel(message)} のメッセージを削除`}
                      title="強制削除"
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
            <AlertDialogTitle>メッセージを強制削除しますか？</AlertDialogTitle>
            <AlertDialogDescription>
              {target
                ? `${authorLabel(target)} さんのメッセージを削除します。この操作は取り消せません。`
                : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={pending}
              onClick={() => void confirmDelete()}
            >
              {pending ? "削除中…" : "削除"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
