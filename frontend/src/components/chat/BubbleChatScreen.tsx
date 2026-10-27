import { MessageCircleMoreIcon, XIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

import { useBubbleChat } from "../../hooks/useBubbleChat";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import { Textarea } from "../ui/textarea";
import { initials } from "./format";

const MAX_INPUT_HEIGHT = 180;

export function BubbleChatScreen() {
  const { t } = useTranslation();
  const { entries, content, setContent, clearContent } = useBubbleChat(true);
  const { currentUserId } = useCurrentUser();
  const usersQuery = useUsers();
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const me = useMemo(
    () => (usersQuery.data ?? []).find((user) => user.id === currentUserId),
    [usersQuery.data, currentUserId],
  );
  const meLabel = me ? me.display_name || me.user_name : (currentUserId ?? "?");

  const otherEntries = useMemo(
    () =>
      entries.filter((entry) => entry.user_id !== currentUserId && entry.content.trim().length > 0),
    [entries, currentUserId],
  );
  const hasOwnBubble = content.trim().length > 0;
  const isEmpty = otherEntries.length === 0 && !hasOwnBubble;

  useLayoutEffect(() => {
    const element = textareaRef.current;
    if (!element) {
      return;
    }
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_INPUT_HEIGHT)}px`;
    element.style.overflowY = element.scrollHeight > MAX_INPUT_HEIGHT ? "auto" : "hidden";
  }, [content]);

  return (
    <section className="flex h-full min-h-0 flex-col" aria-labelledby="bubble-chat-title">
      <header className="flex items-center gap-3 border-b bg-background/80 px-4 py-3 backdrop-blur supports-backdrop-filter:bg-background/60">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-muted text-brand-muted-foreground">
          <MessageCircleMoreIcon className="size-4" aria-hidden="true" />
        </div>
        <div className="min-w-0 flex-1">
          <h1 id="bubble-chat-title" className="truncate text-sm font-semibold">
            {t("bubbleChat.title")}
          </h1>
          <p className="truncate text-xs text-muted-foreground">{t("bubbleChat.description")}</p>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-4" aria-live="polite">
        {isEmpty ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
            <div className="flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
              <MessageCircleMoreIcon className="size-6" aria-hidden="true" />
            </div>
            <p className="text-sm font-medium">{t("bubbleChat.emptyTitle")}</p>
            <p className="max-w-md text-xs text-muted-foreground">
              {t("bubbleChat.emptyDescription")}
            </p>
          </div>
        ) : (
          <ul className="mx-auto flex max-w-3xl flex-col gap-4">
            {otherEntries.map((entry) => {
              const label = entry.display_name || entry.user_name || entry.user_id;
              return (
                <li
                  key={entry.user_id}
                  data-bubble-side="left"
                  className="flex items-start justify-start gap-3"
                >
                  <Avatar className="mt-1 shrink-0" title={label}>
                    <AvatarFallback>{initials(label)}</AvatarFallback>
                  </Avatar>
                  <div className="flex min-w-0 max-w-[75%] flex-col items-start">
                    <span className="mb-1 max-w-full truncate px-1 text-xs font-medium text-muted-foreground">
                      {label}
                    </span>
                    <div className="rounded-2xl rounded-tl-sm border bg-card px-3.5 py-2.5 text-sm leading-relaxed break-words whitespace-pre-wrap text-card-foreground shadow-xs">
                      {entry.content}
                    </div>
                  </div>
                </li>
              );
            })}

            {hasOwnBubble ? (
              <li data-bubble-side="right" className="flex items-start justify-end gap-3">
                <div className="flex min-w-0 max-w-[75%] flex-col items-end">
                  <span className="mb-1 max-w-full truncate px-1 text-right text-xs font-medium text-muted-foreground">
                    {meLabel}
                  </span>
                  <div className="rounded-2xl rounded-tr-sm bg-brand px-3.5 py-2.5 text-sm leading-relaxed break-words whitespace-pre-wrap text-brand-foreground shadow-xs">
                    {content}
                  </div>
                </div>
                <Avatar className="mt-1 shrink-0" title={meLabel}>
                  <AvatarFallback>{initials(meLabel)}</AvatarFallback>
                </Avatar>
              </li>
            ) : null}
          </ul>
        )}
      </div>

      <div className="shrink-0 border-t px-3 py-3">
        <div className="mx-auto max-w-3xl">
          <div className="flex items-end gap-2 rounded-xl border border-input bg-background p-1.5 transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30 dark:bg-input/30">
            <Textarea
              ref={textareaRef}
              rows={1}
              value={content}
              placeholder={t("bubbleChat.placeholder")}
              aria-label={t("bubbleChat.inputLabel")}
              className="min-h-9 resize-none border-0 bg-transparent px-2 py-1.5 text-base shadow-none [field-sizing:fixed] focus-visible:ring-0 md:text-sm dark:bg-transparent"
              onChange={(event) => setContent(event.currentTarget.value)}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              disabled={content.length === 0}
              aria-label={t("bubbleChat.clear")}
              title={t("bubbleChat.clear")}
              onClick={clearContent}
            >
              <XIcon />
            </Button>
          </div>
          <p className="mt-1.5 px-1 text-[11px] text-muted-foreground">{t("bubbleChat.hint")}</p>
        </div>
      </div>
    </section>
  );
}
