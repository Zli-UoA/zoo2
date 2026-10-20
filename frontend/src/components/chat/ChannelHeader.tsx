import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, HashIcon, LockIcon, UsersIcon } from "lucide-react";

import type { ChannelRow } from "../../lib/electric/types";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";

/** チャンネルヘッダ。名前・説明・メンバー数と Members ボタン、狭幅の戻る。 */
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
        <h2 className="truncate text-sm font-semibold">{channel.channel_name}</h2>
        <p className="truncate text-xs text-muted-foreground">
          {channel.description ?? t("channels.noDescription")}
        </p>
      </div>
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
    </header>
  );
}
