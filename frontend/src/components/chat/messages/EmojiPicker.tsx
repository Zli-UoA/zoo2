import { EmojiPicker as Frimousse } from "frimousse";
import { SearchIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { EMOJIBASE_DATA_URL, SUPPORTED_EMOJI_VERSION } from "../../../lib/emoji/data";
import { resolveEmojiDataLocale } from "../../../lib/emoji/locale";
import { addRecentEmoji, loadRecentEmojis } from "../../../lib/emoji/recents";
import { useLocale } from "../../i18n/LocaleProvider";
import { inputClassName } from "../../ui/input-styles";

const COLUMNS = 8;

const emojiButtonClassName =
  "flex min-w-0 flex-1 items-center justify-center rounded-md p-1 text-xl leading-none outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50";

const NAVIGATION_KEYS = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]);

/**
 * Frimousse is headless, so the picker, list and category headers are styled
 * here with the app's design tokens. Emojibase data is self-hosted under
 * `public/emojibase-data`; frimousse caches it in localStorage after the first
 * load, so only the active locale is fetched.
 */
export function EmojiPicker({ onSelect }: { onSelect: (emoji: string) => void }) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const [recents, setRecents] = useState<string[]>([]);
  const viewportRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setRecents(loadRecentEmojis());
  }, []);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const observer = new MutationObserver(() => {
      const active = viewport.querySelector<HTMLElement>('[role="gridcell"][data-active]');
      if (!active) return;
      if (active.getBoundingClientRect().bottom > viewport.getBoundingClientRect().bottom) {
        active.scrollIntoView({ block: "nearest" });
      }
    });
    observer.observe(viewport, {
      attributes: true,
      attributeFilter: ["data-active"],
      subtree: true,
    });
    return () => observer.disconnect();
  }, []);

  const choose = useCallback(
    (emoji: string) => {
      setRecents(addRecentEmoji(emoji));
      onSelect(emoji);
    },
    [onSelect],
  );

  // Base UI Dialog stops arrow-key propagation at the popup, but frimousse
  // listens on document for grid navigation. Re-dispatch the key on document
  // so arrow keys still move through the emoji grid.
  const forwardNavigationKey = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    if (!NAVIGATION_KEYS.has(event.key)) return;
    const { key, code, location, ctrlKey, shiftKey, altKey, metaKey } = event;
    event.preventDefault();
    queueMicrotask(() => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", {
          key,
          code,
          location,
          ctrlKey,
          shiftKey,
          altKey,
          metaKey,
          bubbles: true,
          cancelable: true,
        }),
      );
    });
  }, []);

  return (
    <Frimousse.Root
      locale={resolveEmojiDataLocale(locale)}
      columns={COLUMNS}
      emojibaseUrl={EMOJIBASE_DATA_URL}
      emojiVersion={SUPPORTED_EMOJI_VERSION}
      onEmojiSelect={(emoji) => choose(emoji.emoji)}
      onKeyDown={forwardNavigationKey}
      className="flex h-full min-h-0 flex-col gap-2"
    >
      {recents.length > 0 ? (
        <div
          role="group"
          aria-label={t("reactions.recent")}
          className="flex shrink-0 flex-wrap gap-1 border-b border-border pb-2"
        >
          {recents.map((emoji) => (
            <button
              key={emoji}
              type="button"
              aria-label={emoji}
              className="flex size-8 items-center justify-center rounded-md text-lg leading-none outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50"
              onClick={() => choose(emoji)}
            >
              {emoji}
            </button>
          ))}
        </div>
      ) : null}
      <div className="flex shrink-0 items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <SearchIcon
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
          />
          <Frimousse.Search
            className={cn(inputClassName(), "w-full pl-8")}
            placeholder={t("reactions.searchEmoji")}
            aria-label={t("reactions.searchEmoji")}
            autoFocus
          />
        </div>
        <Frimousse.SkinToneSelector
          aria-label={t("reactions.skinTone")}
          className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-background text-lg leading-none outline-none hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50"
        />
      </div>
      <Frimousse.Viewport
        ref={viewportRef}
        className="min-h-0 flex-1 rounded-lg border border-border"
      >
        <Frimousse.Loading className="block p-3 text-center text-xs text-muted-foreground">
          {t("reactions.emojiLoading")}
        </Frimousse.Loading>
        <Frimousse.Empty className="block p-3 text-center text-xs text-muted-foreground">
          {t("reactions.noSearchResults")}
        </Frimousse.Empty>
        <Frimousse.List
          components={{
            Row: ({ children, "aria-rowindex": ariaRowIndex, ...props }) => (
              <div
                {...props}
                aria-rowindex={ariaRowIndex === undefined ? undefined : Number(ariaRowIndex) + 1}
                className="flex w-full gap-1 px-1"
              >
                {children}
              </div>
            ),
            CategoryHeader: ({ category, ...props }) => (
              <div
                {...props}
                className="flex items-center bg-popover px-2 text-xs font-medium text-muted-foreground"
              >
                {category.label}
              </div>
            ),
            Emoji: ({ emoji, "aria-colindex": ariaColIndex, ...props }) => (
              <button
                {...props}
                aria-colindex={ariaColIndex === undefined ? undefined : Number(ariaColIndex) + 1}
                type="button"
                className={cn(emojiButtonClassName, emoji.isActive && "bg-accent")}
              >
                {emoji.emoji}
              </button>
            ),
          }}
        />
      </Frimousse.Viewport>
    </Frimousse.Root>
  );
}
