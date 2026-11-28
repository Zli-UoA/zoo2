import { Link } from "@tanstack/react-router";
import { useRef, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowUpRightIcon,
  FileTextIcon,
  HashIcon,
  MessageSquareTextIcon,
  PaperclipIcon,
  UserRoundIcon,
  UsersIcon,
} from "lucide-react";

import type { ChannelUserRow, UserRow } from "../../../lib/electric/types";
import { formatAttachmentSize } from "../../../lib/attachments";
import { highlightText } from "../../ui/highlighted-text";
import { useLocale } from "../../i18n/LocaleProvider";
import { useShell } from "../../layout/shell-context";
import { AppAvatar } from "../AppIdentity";
import { formatDmDisplayName, isGroupDm, resolveDmDisplayName } from "../dm";
import { MfmContent } from "../messages/MfmContent";
import {
  NotificationAvatar,
  NotificationCardFrame,
  NotificationTimestamp,
} from "../notifications/NotificationCard";
import { isFileItem, searchResultLink, type SearchItem } from "./globalSearch";
import { isNestedControlClick } from "../../../lib/rowClick";
import { focusSnippet } from "../../../lib/searchHighlight";

/** 検索結果の 1 件。通知カードと同じ枠・アバター・日時で、行のどこを押しても開く。 */
export function SearchResultRow({
  item,
  users,
  members = [],
  currentUserId,
  terms = [],
}: {
  item: SearchItem;
  /** 強調する検索語（`searchTerms`）。本文の抜粋とファイル名の中で当たった部分を強調する。 */
  terms?: readonly string[];
  /** DM の表示名と、投稿者が `display_data` にない古い行の補完に使う。 */
  users: ReadonlyMap<string, UserRow>;
  members?: readonly Pick<ChannelUserRow, "user_id">[];
  currentUserId: string | null;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const shell = useShell();
  const destinationRef = useRef<HTMLAnchorElement>(null);
  const file = isFileItem(item) ? item : null;
  const message = isFileItem(item) ? null : item;
  const fallbackUser = item.user_id ? users.get(item.user_id) : undefined;
  const author = item.author ?? fallbackUser;
  const authorName = author?.display_name || author?.user_name || item.user_id || "?";
  const isBot = !!author?.is_bot;
  const channelLabel = item.is_dm
    ? formatDmDisplayName(
        resolveDmDisplayName({
          channelName: item.channel_name,
          members,
          users: [...users.values()],
          currentUserId,
          locale,
        }),
        t,
      )
    : `#${item.channel_name}`;
  const ChannelIcon = item.is_dm
    ? isGroupDm(item.channel_name)
      ? UsersIcon
      : UserRoundIcon
    : HashIcon;
  // チャンネルは # のアイコンを付けるので、表示する名前には # を重ねない。
  const channelText = item.is_dm ? channelLabel : item.channel_name;
  const link = searchResultLink(item);
  const summary = file ? file.file_name : item.snippet;

  function openFromRow(event: ReactMouseEvent<HTMLLIElement>) {
    // 通知カードと同じく、本物のリンクをキーボードの対象に残し、選択中の文字やリンクは邪魔しない。
    if (
      event.defaultPrevented ||
      isNestedControlClick(event) ||
      (window.getSelection()?.toString().length ?? 0) > 0
    )
      return;
    const destination = destinationRef.current;
    if (!destination) return;
    if (event.ctrlKey || event.metaKey || event.shiftKey) {
      window.open(
        destination.href,
        "_blank",
        event.shiftKey ? "popup,noopener,noreferrer" : "noopener,noreferrer",
      );
      return;
    }
    destination.click();
  }

  return (
    <NotificationCardFrame
      aria-label={`${authorName} — ${channelLabel}: ${summary}`}
      onClick={openFromRow}
      className="cursor-pointer"
      data-search-result={item.id}
    >
      <div className="flex items-start gap-3">
        <NotificationAvatar
          badge={
            file ? (
              <FileTextIcon className="size-3" aria-hidden="true" />
            ) : (
              <MessageSquareTextIcon className="size-3" aria-hidden="true" />
            )
          }
        >
          <AppAvatar label={authorName} title={authorName} isBot={isBot} src={author?.avatar_url} />
        </NotificationAvatar>
        <div className="min-w-0 flex-1 space-y-2 sm:space-y-1">
          <div className="flex min-w-0 items-center gap-2">
            <div className="flex min-w-0 flex-1 items-baseline gap-2">
              <p className="min-w-0 truncate text-sm font-semibold" title={authorName}>
                {authorName}
              </p>
              {isBot ? (
                <span className="rounded bg-muted px-1 text-2xs font-medium text-muted-foreground">
                  APP
                </span>
              ) : null}
            </div>
            <NotificationTimestamp createdAt={item.created_at} />
          </div>
          {file ? (
            <p className="flex min-w-0 items-center gap-1.5 text-sm">
              <PaperclipIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 break-all font-medium">
                {highlightText(file.file_name, terms)}
              </span>
              <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                {formatAttachmentSize(file.file_size, locale)}
              </span>
            </p>
          ) : message ? (
            <blockquote>
              <div className="mfm-body line-clamp-3 whitespace-pre-wrap break-words text-sm leading-relaxed">
                <MfmContent
                  // 狭い画面の 3 行でも最初の一致が見えるよう、その前を縮める。
                  content={focusSnippet(message.snippet, terms)}
                  emojis={message.emojis ?? {}}
                  highlight={terms}
                />
              </div>
            </blockquote>
          ) : null}
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
            <Link
              ref={destinationRef}
              to="/channels/$channelId"
              params={{ channelId: link.channelId }}
              search={link.search}
              onClick={() => shell.showChat()}
              title={channelLabel}
              aria-label={`${channelLabel} — ${authorName}: ${summary}`}
              className="block w-fit max-w-full min-w-0 rounded-sm outline-none hover:underline hover:underline-offset-4 focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                <ChannelIcon className="size-4 shrink-0" aria-hidden="true" />
                <span className="min-w-0 truncate font-medium">{channelText}</span>
                <ArrowUpRightIcon className="size-3.5 shrink-0" aria-hidden="true" />
              </span>
            </Link>
            {item.parent_id ? (
              <span className="rounded bg-muted px-1.5 py-0.5 text-micro font-medium text-muted-foreground">
                {t("globalSearch.inThread")}
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </NotificationCardFrame>
  );
}
