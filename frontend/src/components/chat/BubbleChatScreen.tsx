import { MessageCircleMoreIcon, UsersIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { useBubbleChat } from "../../hooks/useBubbleChat";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { useMediaQuery } from "../layout/useMediaQuery";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import { ScrollArea } from "../ui/scroll-area";
import { Textarea } from "../ui/textarea";
import { initials } from "./format";

const MAX_INPUT_HEIGHT = 180;
const POP_DURATION_MS = 3500;

export interface BubbleChatKeyInput {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey?: boolean;
  isComposing: boolean;
}

/**
 * Cmd+Enter / Ctrl+Enter で入力中の文字列を全削除する。
 * Enter / Shift+Enter は改行に任せ、IME 変換中（isComposing）は実行しない。
 */
export function shouldClearBubbleChatFromKey(input: BubbleChatKeyInput): boolean {
  if (input.key !== "Enter" || input.isComposing || input.shiftKey || input.altKey) {
    return false;
  }
  return input.metaKey || input.ctrlKey;
}

export function BubbleChatScreen() {
  const { t } = useTranslation();
  const { entries, content, isSharing, setContent, clearContent } = useBubbleChat(true);
  const { currentUserId } = useCurrentUser();
  const usersQuery = useUsers();
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);
  const isDesktop = useMediaQuery("(min-width: 768px)");
  const [isParticipantsOpen, setIsParticipantsOpen] = useState(false);
  const [poppingBubble, setPoppingBubble] = useState<{ content: string; id: number } | null>(null);
  const popTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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
  const hasOwnBubble = isSharing && content.trim().length > 0 && poppingBubble === null;
  const isEmpty = otherEntries.length === 0 && !hasOwnBubble && !poppingBubble;

  // 参加者一覧データ（自分＋他のアクティブな参加者）
  const participants = useMemo(() => {
    const list: Array<{
      id: string;
      displayName: string;
      userName: string;
      isSelf: boolean;
      isTyping: boolean;
    }> = [
      {
        id: currentUserId ?? "me",
        displayName: me?.display_name || me?.user_name || t("bubbleChat.you"),
        userName: me?.user_name || "",
        isSelf: true,
        isTyping: hasOwnBubble || poppingBubble !== null,
      },
    ];

    for (const entry of otherEntries) {
      if (!list.some((item) => item.id === entry.user_id)) {
        list.push({
          id: entry.user_id,
          displayName: entry.display_name || entry.user_name || entry.user_id,
          userName: entry.user_name || "",
          isSelf: false,
          isTyping: true,
        });
      }
    }
    return list;
  }, [currentUserId, me, otherEntries, hasOwnBubble, poppingBubble, t]);

  const triggerBubblePop = useCallback(() => {
    if (popTimerRef.current) {
      clearTimeout(popTimerRef.current);
      popTimerRef.current = null;
    }
    const current = content.trim();
    if (current.length > 0) {
      setContent("");
      setPoppingBubble({ content: current, id: Date.now() });
      popTimerRef.current = setTimeout(() => {
        popTimerRef.current = null;
        setPoppingBubble(null);
        clearContent();
      }, POP_DURATION_MS);
    } else {
      clearContent();
      setPoppingBubble(null);
    }
  }, [content, setContent, clearContent]);

  const handleContentChange = useCallback(
    (value: string) => {
      if (popTimerRef.current) {
        clearTimeout(popTimerRef.current);
        popTimerRef.current = null;
        setPoppingBubble(null);
      }
      setContent(value);
    },
    [setContent],
  );

  useEffect(() => {
    return () => {
      if (popTimerRef.current) {
        clearTimeout(popTimerRef.current);
      }
    };
  }, []);

  useLayoutEffect(() => {
    const element = textareaRef.current;
    if (!element) {
      return;
    }
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_INPUT_HEIGHT)}px`;
    element.style.overflowY = element.scrollHeight > MAX_INPUT_HEIGHT ? "auto" : "hidden";
  }, [content]);

  const participantsListContent = (
    <ul className="flex flex-col gap-1 p-2">
      {participants.map((p) => (
        <li
          key={p.id}
          className="flex items-center gap-2.5 rounded-lg px-2.5 py-2 transition-colors hover:bg-accent/60"
        >
          <Avatar className="size-8 shrink-0" title={p.displayName}>
            <AvatarFallback>{initials(p.displayName)}</AvatarFallback>
          </Avatar>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span className="truncate text-xs font-semibold">{p.displayName}</span>
              {p.isSelf ? (
                <span className="shrink-0 rounded bg-muted px-1.5 py-0.2 text-[10px] font-medium text-muted-foreground">
                  {t("bubbleChat.you")}
                </span>
              ) : null}
            </div>
            {p.userName ? (
              <span className="block truncate text-[11px] text-muted-foreground">
                @{p.userName}
              </span>
            ) : null}
          </div>
          <div className="shrink-0">
            {p.isTyping ? (
              <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                <span className="size-1.5 animate-pulse rounded-full bg-emerald-500" />
                {t("bubbleChat.typing")}
              </span>
            ) : (
              <span className="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
                {t("bubbleChat.active")}
              </span>
            )}
          </div>
        </li>
      ))}
    </ul>
  );

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
        <Button
          variant={isParticipantsOpen ? "secondary" : "outline"}
          size="sm"
          aria-label={`${t("bubbleChat.participants")} ${participants.length}`}
          title={t("bubbleChat.participantsTitle", { count: participants.length })}
          className="gap-1.5"
          onClick={() => setIsParticipantsOpen((prev) => !prev)}
        >
          <UsersIcon className="size-4" aria-hidden="true" />
          <span className="text-xs font-semibold">{participants.length}</span>
        </Button>
      </header>

      <div className="sr-only" aria-live="polite" aria-atomic="true">
        {otherEntries.length > 0
          ? t("bubbleChat.typingSummary", {
              count: otherEntries.length,
              names: otherEntries.map((e) => e.display_name || e.user_name).join(", "),
            })
          : t("bubbleChat.emptyTitle")}
      </div>

      <div className="flex min-h-0 flex-1 flex-row overflow-hidden">
        <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
          <div className="min-h-0 flex-1 overflow-y-auto p-4">
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

                {hasOwnBubble || poppingBubble ? (
                  <li
                    key={poppingBubble ? `pop-${poppingBubble.id}` : "active"}
                    data-bubble-side="right"
                    className={cn(
                      "flex items-start justify-end gap-3",
                      poppingBubble && !hasOwnBubble && "animate-bubble-pop",
                    )}
                  >
                    <div className="flex min-w-0 max-w-[75%] flex-col items-end">
                      <span className="mb-1 max-w-full truncate px-1 text-right text-xs font-medium text-muted-foreground">
                        {meLabel}
                      </span>
                      <div
                        className={cn(
                          "rounded-2xl rounded-tr-sm bg-brand px-3.5 py-2.5 text-sm leading-relaxed break-words whitespace-pre-wrap text-brand-foreground shadow-xs",
                          poppingBubble && !hasOwnBubble && "shadow-md ring-2 ring-brand/40",
                        )}
                      >
                        {hasOwnBubble ? content : poppingBubble?.content}
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
                  onCompositionStart={() => {
                    composingRef.current = true;
                  }}
                  onCompositionEnd={() => {
                    composingRef.current = false;
                  }}
                  onChange={(event) => handleContentChange(event.currentTarget.value)}
                  onKeyDown={(event) => {
                    if (
                      composingRef.current ||
                      event.nativeEvent.isComposing ||
                      event.nativeEvent.keyCode === 229
                    ) {
                      return;
                    }
                    if (
                      shouldClearBubbleChatFromKey({
                        key: event.key,
                        metaKey: event.metaKey,
                        ctrlKey: event.ctrlKey,
                        shiftKey: event.shiftKey,
                        altKey: event.altKey,
                        isComposing: event.nativeEvent.isComposing,
                      })
                    ) {
                      event.preventDefault();
                      triggerBubblePop();
                    }
                  }}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  disabled={content.length === 0}
                  aria-label={t("bubbleChat.clear")}
                  title={t("bubbleChat.clear")}
                  onClick={triggerBubblePop}
                >
                  <XIcon />
                </Button>
              </div>
              <p className="mt-1.5 px-1 text-[11px] text-muted-foreground">
                {t("bubbleChat.hint")}
              </p>
            </div>
          </div>
        </div>

        {/* 参加者サイドパネル（デスクトップ） */}
        {isDesktop && isParticipantsOpen ? (
          <aside
            aria-label={t("bubbleChat.participants")}
            className="flex w-72 shrink-0 flex-col border-l bg-background"
          >
            <div className="flex items-center justify-between border-b px-4 py-3">
              <h2 className="text-sm font-semibold">
                {t("bubbleChat.participantsTitle", { count: participants.length })}
              </h2>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={t("bubbleChat.closeParticipants")}
                onClick={() => setIsParticipantsOpen(false)}
              >
                <XIcon className="size-4" />
              </Button>
            </div>
            <ScrollArea className="flex-1">{participantsListContent}</ScrollArea>
          </aside>
        ) : null}
      </div>

      {/* 参加者ダイアログ（モバイル・狭幅） */}
      {!isDesktop ? (
        <Dialog open={isParticipantsOpen} onOpenChange={setIsParticipantsOpen}>
          <DialogContent className="max-h-[80dvh] flex flex-col p-0 sm:max-w-xs">
            <DialogHeader className="border-b px-4 py-3">
              <DialogTitle className="text-sm">
                {t("bubbleChat.participantsTitle", { count: participants.length })}
              </DialogTitle>
            </DialogHeader>
            <ScrollArea className="flex-1">{participantsListContent}</ScrollArea>
          </DialogContent>
        </Dialog>
      ) : null}
    </section>
  );
}
