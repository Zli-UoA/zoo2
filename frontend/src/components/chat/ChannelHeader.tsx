import { ChannelWebhooksDialog } from "./ChannelWebhooksDialog";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ArrowLeftIcon,
  HashIcon,
  LockIcon,
  LogOutIcon,
  MessagesSquareIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SettingsIcon,
  WebhookIcon,
  UsersIcon,
} from "lucide-react";
import { useState } from "react";

import { useChannelAdmin } from "../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelRow } from "../../lib/electric/types";
import type { ChannelUserWriteMetadata } from "../../lib/electric/writes";
import { localizeApiError } from "../../lib/i18n/apiError";
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
import { SettingRow, SettingsSection } from "../ui/control-panel";
import { Button, buttonVariants } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { normalizeChannelDescription } from "./channelDescription";
import { RenameChannelDialog } from "./RenameChannelDialog";
import { isGroupDm } from "./dm";

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
  const [settingsOpen, setSettingsOpen] = useState(false);

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
    <header className="flex min-h-16 shrink-0 items-center gap-3 border-b bg-background/80 px-4 py-2.5 backdrop-blur supports-backdrop-filter:bg-background/60">
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-11 md:hidden"
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
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-11 md:size-8"
            aria-label={t("settings.channelTitle")}
            title={t("settings.channelTitle")}
            onClick={() => setSettingsOpen(true)}
          >
            <SettingsIcon />
          </Button>
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

      <Dialog open={settingsOpen && canManage} onOpenChange={setSettingsOpen}>
        <DialogContent className="max-h-[85dvh] overflow-y-auto sm:max-w-lg">
          <DialogHeader className="pr-8">
            <DialogTitle>{t("settings.channelTitle")}</DialogTitle>
            <DialogDescription>{displayTitle}</DialogDescription>
          </DialogHeader>
          <div className="divide-y rounded-lg border">
            <SettingRow label={t("channels.editTitle")} description={displaySubtitle}>
              <Button variant="outline" disabled={isArchived} onClick={() => setRenameOpen(true)}>
                <PencilIcon />
                {t("messages.edit")}
              </Button>
            </SettingRow>
            <SettingRow label={t("webhooks.title")}>
              <Button variant="outline" onClick={() => setWebhooksOpen(true)}>
                <WebhookIcon />
                {t("common.open")}
              </Button>
            </SettingRow>
          </div>
          <SettingsSection className="border-destructive/30">
            <h3 className="font-medium">
              {isArchived ? t("moderation.unarchive") : t("moderation.archive")}
            </h3>
            <p className="mt-1 text-xs text-muted-foreground">
              {isArchived
                ? t("moderation.unarchiveDescription")
                : t("moderation.archiveDescription")}
            </p>
            <Button
              className="mt-3"
              variant={isArchived ? "outline" : "destructive"}
              onClick={() => setConfirmAction(isArchived ? "unarchive" : "archive")}
            >
              {isArchived ? <ArchiveRestoreIcon /> : <ArchiveIcon />}
              {isArchived ? t("moderation.unarchive") : t("moderation.archive")}
            </Button>
          </SettingsSection>
        </DialogContent>
      </Dialog>
    </header>
  );
}

function DmHeaderMenu({ channel }: { channel: ChannelRow }) {
  const { t } = useTranslation();
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
