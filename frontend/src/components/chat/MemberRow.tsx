import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { AppAvatar, openApp } from "./AppIdentity";

/**
 * メンバー一覧の1行。チャンネルのメンバー（`ChannelMembersDialog`）とスレッドのメンバー
 * （`ThreadParticipantsDialog`）で、アバター・名前・「作成者」「あなた」の印をそろえる。
 */
export function MemberRow({
  label,
  avatarUrl,
  isBot = false,
  appId,
  creatorTag,
  isMe = false,
  badge,
  actions,
}: {
  label: string;
  avatarUrl?: string | null;
  isBot?: boolean;
  /** Bot が App のときの ID。名前を押すと App の画面を開く。 */
  appId?: string | null;
  /** 作成者の印の文言（チャンネルは「作成者」、スレッドは「スレ主」）。作成者でなければ省く。 */
  creatorTag?: string;
  isMe?: boolean;
  /** 名前と作成者の印の間に出す、そのほかの印（暗号化の警告など）。 */
  badge?: ReactNode;
  /** 行の右端の操作（キックなど）。 */
  actions?: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <li className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-accent/60">
      <AppAvatar label={label} isBot={isBot} src={avatarUrl} size="sm" />
      <span className="min-w-0 flex-1 truncate text-sm">
        {isBot && appId ? (
          <button type="button" onClick={() => openApp(appId)}>
            {label} <span className="text-xs text-muted-foreground">APP</span>
          </button>
        ) : (
          label
        )}
      </span>
      {badge}
      {creatorTag ? (
        <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-2xs text-muted-foreground">
          {creatorTag}
        </span>
      ) : null}
      {isMe ? (
        <span className="shrink-0 rounded bg-brand-muted px-1.5 py-0.5 text-2xs text-brand-muted-foreground">
          {t("members.you")}
        </span>
      ) : null}
      {actions}
    </li>
  );
}
