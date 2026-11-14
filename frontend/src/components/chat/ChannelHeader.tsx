import { ChannelWebhooksDialog } from "./ChannelWebhooksDialog";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ArrowLeftIcon,
  HashIcon,
  HeadphonesIcon,
  LockIcon,
  LockKeyholeIcon,
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
import { useCall } from "../../lib/call/call-context";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelRow } from "../../lib/electric/types";
import { parseUtcTimestamp } from "../../lib/timestamps";
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
import { EncryptChannelDialog } from "./EncryptChannelDialog";
import { isGroupDm } from "./dm";
import { PaneHeader } from "../layout/PaneHeader";

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
  const { t, i18n } = useTranslation();
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
  const [encryptOpen, setEncryptOpen] = useState(false);
  // 既存の非公開チャンネルは、管理できる人が暗号化に切り替えられる（#367）。
  const canEncrypt = canManage && channel.is_private && !channel.encrypted && !isArchived;

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
    <PaneHeader className="bg-background/80 backdrop-blur supports-backdrop-filter:bg-background/60">
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-11 md:hidden"
        aria-label={t("channels.back")}
        onClick={shell.showSidebar}
      >
        <ArrowLeftIcon />
      </Button>
      <div className="relative flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
        {isDm ? (
          <>
            {isGroup ? (
              <UsersIcon className="size-4" aria-hidden="true" />
            ) : (
              <MessagesSquareIcon className="size-4" aria-hidden="true" />
            )}
            {channel.encrypted ? (
              // DM はチャンネル設定を開けないので、暗号化 DM はアイコンに鍵を重ねて見分けられるようにする。
              <LockKeyholeIcon
                className="absolute -right-1 -bottom-1 size-3.5 rounded-full bg-background p-0.5 text-foreground ring-1 ring-border"
                aria-hidden="true"
              />
            ) : null}
          </>
        ) : channel.encrypted ? (
          <LockKeyholeIcon className="size-4" aria-hidden="true" />
        ) : channel.is_private ? (
          <LockIcon className="size-4" aria-hidden="true" />
        ) : (
          <HashIcon className="size-4" aria-hidden="true" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <h2 className="truncate text-sm font-semibold">{displayTitle}</h2>
          {channel.encrypted ? (
            // 見た目の印は左のアイコン（DM は鍵を重ねたアイコン）だけにし、印の文字はチャンネル設定に出す。読み上げでは、
            // 送信の前に暗号化されているかを確かめられるよう、見出しの隣で伝える。
            <span className="sr-only">{t("e2ee.badge")}</span>
          ) : null}
          {channel.is_nsfw ? (
            <span className="shrink-0 text-xs text-muted-foreground">NSFW</span>
          ) : null}
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

      {canEncrypt ? (
        <EncryptChannelDialog channel={channel} open={encryptOpen} onOpenChange={setEncryptOpen} />
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
          {/* 狭い画面では、印が見出しの下へ折り返す (見出しの幅を 10rem は確保する) */}
          <div className="flex flex-wrap items-start justify-between gap-2 pr-8">
            <DialogHeader className="min-w-40 flex-1">
              <DialogTitle>{t("settings.channelTitle")}</DialogTitle>
              <DialogDescription>{displayTitle}</DialogDescription>
            </DialogHeader>
            {channel.encrypted || channel.imported_from_slack ? (
              <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                {channel.encrypted ? (
                  <span className="inline-flex items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground">
                    <LockKeyholeIcon className="size-3" aria-hidden="true" />
                    {t("e2ee.badge")}
                  </span>
                ) : null}
                {channel.imported_from_slack ? (
                  <span className="rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground">
                    {slackImportBadge(channel, t, i18n.language)}
                  </span>
                ) : null}
              </div>
            ) : null}
          </div>
          <div className="divide-y rounded-lg border">
            <SettingRow label={t("channels.editTitle")} description={displaySubtitle}>
              <Button
                variant="outline"
                disabled={isArchived}
                onClick={() => {
                  setSettingsOpen(false);
                  setRenameOpen(true);
                }}
              >
                <PencilIcon />
                {t("messages.edit")}
              </Button>
            </SettingRow>
            {canEncrypt ? (
              <SettingRow
                label={t("e2ee.convert.action")}
                description={t("e2ee.convert.settingHelp")}
              >
                <Button
                  variant="outline"
                  onClick={() => {
                    setSettingsOpen(false);
                    setEncryptOpen(true);
                  }}
                >
                  <LockKeyholeIcon />
                  {t("e2ee.convert.start")}
                </Button>
              </SettingRow>
            ) : null}
            <SettingRow label={t("webhooks.title")}>
              <Button
                variant="outline"
                onClick={() => {
                  setSettingsOpen(false);
                  setWebhooksOpen(true);
                }}
              >
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
    </PaneHeader>
  );
}

function DmHeaderMenu({ channel }: { channel: ChannelRow }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { channelUsers } = useCollections();
  const { currentUserId } = useCurrentUser();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [encryptOpen, setEncryptOpen] = useState(false);
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
          {!channel.encrypted ? (
            // 既存の DM も、参加者なら暗号化に切り替えられる（#367）。
            <DropdownMenuItem className="min-h-11" onClick={() => setEncryptOpen(true)}>
              <LockKeyholeIcon />
              {t("e2ee.convert.actionDm")}
            </DropdownMenuItem>
          ) : null}
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

      {!channel.encrypted ? (
        <EncryptChannelDialog channel={channel} open={encryptOpen} onOpenChange={setEncryptOpen} />
      ) : null}

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

/** チャンネル設定の右上に出す小さな印。取り込みの種類 (最初の取り込み) と最後に取り込んだ日付がわかるときは、それも出す */
function slackImportBadge(channel: ChannelRow, t: TFunction, language: string): string {
  if (channel.slack_import_kind && channel.slack_imported_at) {
    const date = parseUtcTimestamp(channel.slack_imported_at).toLocaleDateString(language);
    return t(
      channel.slack_import_kind === "created"
        ? "channels.slackImportedCreated"
        : "channels.slackImportedExisting",
      { date },
    );
  }
  return t("channels.slackImported");
}
