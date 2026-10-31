import { EraserIcon, MessageCircleMoreIcon, UsersIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { useBubbleChat } from "../../hooks/useBubbleChat";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { useMediaQuery } from "../layout/useMediaQuery";
import { AppAvatar } from "./AppIdentity";
import { ComposerField, composerTextareaClassName } from "./messages/ComposerField";
import { useAutosizeTextarea } from "../../hooks/useAutosizeTextarea";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import { ScrollArea } from "../ui/scroll-area";
import { Textarea } from "../ui/textarea";

const MAX_INPUT_HEIGHT = 180;
export const FADE_DURATION_MS = 3000;
export const POP_DURATION_MS = FADE_DURATION_MS;

interface DisplayedOtherEntry {
  userId: string;
  userName: string;
  displayName: string;
  content: string;
  isFadingOut: boolean;
}

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
  const isTouch = useMediaQuery("(hover: none)");
  const [isParticipantsOpen, setIsParticipantsOpen] = useState(false);
  const [fadingBubble, setFadingBubble] = useState<{ content: string; id: number } | null>(null);
  const fadeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const usersById = useMemo(
    () => new Map((usersQuery.data ?? []).map((user) => [user.id, user])),
    [usersQuery.data],
  );
  const me = usersById.get(currentUserId ?? "");
  const meLabel = me ? me.display_name || me.user_name : (currentUserId ?? "?");

  const otherEntries = useMemo(
    () =>
      entries.filter((entry) => entry.user_id !== currentUserId && entry.content.trim().length > 0),
    [entries, currentUserId],
  );

  const [displayedOthers, setDisplayedOthers] = useState<DisplayedOtherEntry[]>(() =>
    otherEntries.map((entry) => ({
      userId: entry.user_id,
      userName: entry.user_name,
      displayName: entry.display_name,
      content: entry.content,
      isFadingOut: false,
    })),
  );
  const fadingTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  // 他のユーザーの吹き出しが消去された際、即座に消さずにフェードアウトアニメーションを適用する
  useEffect(() => {
    setDisplayedOthers((prev) => {
      const next: DisplayedOtherEntry[] = [];
      const activeUserIds = new Set<string>();

      // 1. 現在アクティブな otherEntries
      for (const entry of otherEntries) {
        activeUserIds.add(entry.user_id);
        const existingTimer = fadingTimersRef.current.get(entry.user_id);
        if (existingTimer) {
          clearTimeout(existingTimer);
          fadingTimersRef.current.delete(entry.user_id);
        }
        next.push({
          userId: entry.user_id,
          userName: entry.user_name,
          displayName: entry.display_name,
          content: entry.content,
          isFadingOut: false,
        });
      }

      // 2. 直前の表示エントリのうち、otherEntries から消えたものをフェードアウト状態として維持
      for (const prevEntry of prev) {
        if (!activeUserIds.has(prevEntry.userId)) {
          if (prevEntry.isFadingOut) {
            next.push(prevEntry);
          } else {
            const fadingEntry: DisplayedOtherEntry = {
              ...prevEntry,
              isFadingOut: true,
            };
            next.push(fadingEntry);

            const timer = setTimeout(() => {
              fadingTimersRef.current.delete(prevEntry.userId);
              setDisplayedOthers((current) =>
                current.filter((item) => item.userId !== prevEntry.userId),
              );
            }, FADE_DURATION_MS);
            fadingTimersRef.current.set(prevEntry.userId, timer);
          }
        }
      }

      return next;
    });
  }, [otherEntries]);

  useEffect(() => {
    return () => {
      fadingTimersRef.current.forEach((timer) => clearTimeout(timer));
      fadingTimersRef.current.clear();
      if (fadeTimerRef.current) {
        clearTimeout(fadeTimerRef.current);
      }
    };
  }, []);

  const hasOwnBubble = isSharing && content.trim().length > 0 && fadingBubble === null;
  const hasOtherBubbles = displayedOthers.length > 0;
  const isEmpty = !hasOtherBubbles && !hasOwnBubble && !fadingBubble;

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
        isTyping: hasOwnBubble || fadingBubble !== null,
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
  }, [currentUserId, me, otherEntries, hasOwnBubble, fadingBubble, t]);

  const triggerBubbleFade = useCallback(() => {
    if (fadeTimerRef.current) {
      clearTimeout(fadeTimerRef.current);
      fadeTimerRef.current = null;
    }
    const current = content.trim();
    if (current.length > 0) {
      setContent("");
      setFadingBubble({ content: current, id: Date.now() });
      fadeTimerRef.current = setTimeout(() => {
        fadeTimerRef.current = null;
        setFadingBubble(null);
        clearContent();
      }, FADE_DURATION_MS);
    } else {
      clearContent();
      setFadingBubble(null);
    }
  }, [content, setContent, clearContent]);

  const handleContentChange = useCallback(
    (value: string) => {
      if (fadeTimerRef.current) {
        clearTimeout(fadeTimerRef.current);
        fadeTimerRef.current = null;
        setFadingBubble(null);
      }
      setContent(value);
    },
    [setContent],
  );

  useAutosizeTextarea(textareaRef, content, MAX_INPUT_HEIGHT);

  const participantsListContent = (
    <ul className="flex flex-col gap-1 p-2">
      {participants.map((p) => (
        <li
          key={p.id}
          className="flex items-center gap-2.5 rounded-lg px-2.5 py-2 transition-colors hover:bg-accent/60"
        >
          <AppAvatar
            className="size-8 shrink-0"
            title={p.displayName}
            label={p.displayName}
            src={usersById.get(p.id)?.avatar_url}
          />
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
              <span className="inline-flex items-center gap-1 rounded-full bg-brand-muted px-2 py-0.5 text-[10px] font-medium text-brand-muted-foreground">
                <span className="size-1.5 animate-pulse rounded-full bg-brand motion-reduce:animate-none" />
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
                {displayedOthers.map((entry) => {
                  const label = entry.displayName || entry.userName || entry.userId;
                  return (
                    <li
                      key={entry.userId}
                      data-bubble-side="left"
                      className={cn(
                        "flex items-start justify-start gap-3",
                        entry.isFadingOut && "animate-bubble-fade-out",
                      )}
                    >
                      <AppAvatar
                        className="mt-1 shrink-0"
                        title={label}
                        label={label}
                        src={usersById.get(entry.userId)?.avatar_url}
                      />
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

                {hasOwnBubble || fadingBubble ? (
                  <li
                    key={fadingBubble ? `fade-${fadingBubble.id}` : "active"}
                    data-bubble-side="right"
                    className={cn(
                      "flex items-start justify-end gap-3",
                      fadingBubble && !hasOwnBubble && "animate-bubble-fade-out",
                    )}
                  >
                    <div className="flex min-w-0 max-w-[75%] flex-col items-end">
                      <span className="mb-1 max-w-full truncate px-1 text-right text-xs font-medium text-muted-foreground">
                        {meLabel}
                      </span>
                      <div className="rounded-2xl rounded-tr-sm bg-brand px-3.5 py-2.5 text-sm leading-relaxed break-words whitespace-pre-wrap text-brand-foreground shadow-xs">
                        {hasOwnBubble ? content : fadingBubble?.content}
                      </div>
                    </div>
                    <AppAvatar
                      className="mt-1 shrink-0"
                      title={meLabel}
                      label={meLabel}
                      src={me?.avatar_url}
                    />
                  </li>
                ) : null}
              </ul>
            )}
          </div>

          <div className="shrink-0 border-t px-3 py-3">
            <div className="mx-auto max-w-3xl">
              <ComposerField>
                <Textarea
                  ref={textareaRef}
                  rows={1}
                  value={content}
                  placeholder={t("bubbleChat.placeholder")}
                  aria-label={t("bubbleChat.inputLabel")}
                  className={`${composerTextareaClassName} min-h-9 px-2 py-1.5`}
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
                      triggerBubbleFade();
                    }
                  }}
                />
                <Button
                  type="button"
                  size="icon-lg"
                  className="mb-1 size-11! md:size-9!"
                  disabled={content.trim().length === 0}
                  aria-label={t("bubbleChat.clear")}
                  title={t("bubbleChat.clear")}
                  onClick={triggerBubbleFade}
                >
                  <EraserIcon className="size-5 md:size-4" />
                </Button>
              </ComposerField>
              {!isTouch ? (
                <p className="mt-1.5 px-1 text-[11px] text-muted-foreground">
                  {t("bubbleChat.hint")}
                </p>
              ) : null}
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
