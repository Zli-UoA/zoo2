import { AccountGlyph, useMfmAccounts } from "./MfmAccounts";
import { useWorkspaceEmojis } from "../../../lib/emoji/workspace";
import { Button } from "../../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { noAutofill } from "../../ui/autofill";
import { EmojiPicker as Frimousse } from "frimousse";
import { SearchIcon } from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { EMOJIBASE_DATA_URL, SUPPORTED_EMOJI_VERSION } from "../../../lib/emoji/data";
import { resolveEmojiDataLocale } from "../../../lib/emoji/locale";
import { resolveMultilingualEmojiData } from "../../../lib/emoji/resolver";
import { useLocale } from "../../i18n/LocaleProvider";
import { inputClassName } from "../../ui/input-styles";

const COLUMNS = 8;

const emojiButtonClassName =
  "flex min-w-0 flex-1 items-center justify-center rounded-md p-1 text-xl leading-none outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50";

const NAVIGATION_KEYS = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]);
const SCROLL_CORRECTION_WINDOW_MS = 200;

/**
 * Frimousse is headless, so the picker, list and category headers are styled
 * here with the app's design tokens. Emojibase data is self-hosted under
 * `public/emojibase-data`; frimousse caches the active locale's data in
 * localStorage after the first load. `resolveMultilingualEmojiData` also loads
 * `search-index.json` once so every language (and pinyin) is searchable.
 */
const KeyboardNavigationContext = createContext<{ current: number }>({
  current: 0,
});

/** Owns the frimousse state (search, skin tone, keyboard navigation) for its children. */
export function EmojiPickerRoot({
  onSelect,
  className,
  gridVisible = true,
  children,
}: {
  onSelect: (emoji: string) => void;
  className?: string;
  /** False while another tab is showing: keys must not pick a hidden emoji. */
  gridVisible?: boolean;
  children: ReactNode;
}) {
  const { locale } = useLocale();
  const keyboardNavigationAt = useRef(0);

  // Base UI Dialog stops arrow-key propagation at the popup, but frimousse
  // listens on document for grid navigation. Re-dispatch the key on document
  // so arrow keys still move through the emoji grid.
  const forwardNavigationKey = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      const navigation = NAVIGATION_KEYS.has(event.key);
      if (!navigation && event.key !== "Enter") return;
      // Only the shared search box and the grid navigate emoji; other inputs
      // inside the root (e.g. Emoji Kitchen) keep their caret/selection keys.
      if (
        !(event.target instanceof Element) ||
        !event.target.closest("[frimousse-search], [frimousse-viewport]")
      )
        return;
      if (!gridVisible) {
        // frimousse listens on document; keep Enter/arrows from reaching it
        // so they cannot select an emoji that is not on screen.
        event.stopPropagation();
        return;
      }
      if (!navigation) return;
      const { key, code, location, ctrlKey, shiftKey, altKey, metaKey } = event;
      keyboardNavigationAt.current = performance.now();
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
    },
    [keyboardNavigationAt, gridVisible],
  );

  return (
    <KeyboardNavigationContext.Provider value={keyboardNavigationAt}>
      <Frimousse.Root
        locale={resolveEmojiDataLocale(locale)}
        columns={COLUMNS}
        emojibaseUrl={EMOJIBASE_DATA_URL}
        resolveEmojiData={resolveMultilingualEmojiData}
        emojiVersion={SUPPORTED_EMOJI_VERSION}
        onEmojiSelect={(emoji) => onSelect(emoji.emoji)}
        onKeyDown={forwardNavigationKey}
        className={cn("flex h-full min-h-0 flex-col gap-2", className)}
      >
        {children}
      </Frimousse.Root>
    </KeyboardNavigationContext.Provider>
  );
}

/** Search box (plus skin tone selector) driving the surrounding `EmojiPickerRoot`. */
export function EmojiSearchBar({
  query,
  onQueryChange,
}: {
  query: string;
  onQueryChange: (query: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex shrink-0 items-center gap-2">
      <div className="relative min-w-0 flex-1">
        <SearchIcon
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
        />
        <Frimousse.Search
          {...noAutofill}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
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
  );
}

/**
 * The emoji grid. `children` render after the grid inside the same scroller
 * (used to append custom emoji matches while searching); `hideEmpty` suppresses
 * the "no results" message when those extra matches exist.
 */
export function EmojiGrid({
  children,
  hideEmpty = false,
}: {
  children?: ReactNode;
  hideEmpty?: boolean;
}) {
  const { t } = useTranslation();
  const keyboardNavigationAt = useContext(KeyboardNavigationContext);
  const viewportRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const observer = new MutationObserver(() => {
      if (performance.now() - keyboardNavigationAt.current > SCROLL_CORRECTION_WINDOW_MS) return;
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
  }, [keyboardNavigationAt]);

  return (
    <Frimousse.Viewport
      ref={viewportRef}
      className="min-h-0 flex-1 rounded-lg border border-border"
    >
      <Frimousse.Loading className="block p-3 text-center text-xs text-muted-foreground">
        {t("reactions.emojiLoading")}
      </Frimousse.Loading>
      <Frimousse.Empty
        className={cn("block text-center text-xs text-muted-foreground", !hideEmpty && "p-3")}
      >
        {hideEmpty ? null : t("reactions.noSearchResults")}
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
      {children}
    </Frimousse.Viewport>
  );
}

/** Self-contained emoji picker (search + grid) for places without tabs. */
export function EmojiPicker({ onSelect }: { onSelect: (emoji: string) => void }) {
  const [query, setQuery] = useState("");
  return (
    <EmojiPickerRoot onSelect={onSelect}>
      <EmojiSearchBar query={query} onQueryChange={setQuery} />
      <EmojiGrid />
    </EmojiPickerRoot>
  );
}

export type CustomEmojiOption = {
  name: string;
  category: string;
  aliases: string[];
  url: string;
  width?: number | null;
  height?: number | null;
};

/**
 * Lower is better: 0 exact name, 1 name prefix, 2 name substring, then the same three
 * tiers (3 exact, 4 prefix, 5 substring) over aliases and category, which rank equally.
 * Null when nothing matches.
 */
function customEmojiRank(e: CustomEmojiOption, needle: string): number | null {
  const name = e.name.toLowerCase();
  const others = [...(e.aliases ?? []), e.category]
    .map((v) => v?.toLowerCase())
    .filter((v): v is string => Boolean(v));
  if (name === needle) return 0;
  if (name.startsWith(needle)) return 1;
  if (name.includes(needle)) return 2;
  if (others.includes(needle)) return 3;
  if (others.some((v) => v.startsWith(needle))) return 4;
  if (others.some((v) => v.includes(needle))) return 5;
  return null;
}

/** Exact matches first, then prefix, then substring; shorter names first within a rank. */
export function rankCustomEmojis(
  options: CustomEmojiOption[],
  needle: string,
): CustomEmojiOption[] {
  // No query: keep the registration order (every emoji would otherwise rank as a prefix match).
  if (needle === "") return options;
  return options
    .map((e, index) => ({ e, index, rank: customEmojiRank(e, needle) }))
    .filter((x): x is { e: CustomEmojiOption; index: number; rank: number } => x.rank !== null)
    .sort((a, b) => a.rank - b.rank || a.e.name.length - b.e.name.length || a.index - b.index)
    .map((x) => x.e);
}

/** Workspace custom emoji plus account avatars, filtered by `query`. */
export function useCustomEmojiSearch(query: string) {
  const { emojis, loading, failed, retry, imageUrl } = useWorkspaceEmojis();
  const accounts = useMfmAccounts();
  const options: CustomEmojiOption[] = [
    ...emojis,
    ...Array.from(accounts.values(), (account) => ({
      name: `@${account.username}`,
      category: "",
      aliases: [account.display_name],
      url: account.avatar_url ?? "",
    })),
  ];
  const needle = query.trim().toLowerCase();
  const filtered = rankCustomEmojis(options, needle);
  return { filtered, searching: needle !== "", loading, failed, retry, imageUrl };
}

type CustomEmojiSearch = ReturnType<typeof useCustomEmojiSearch>;

/**
 * Reserve picker image dimensions from metadata before decode while still letting the
 * decoded image's natural (post-EXIF) aspect ratio take over once loaded.
 */
function customEmojiAspectRatio(emoji: CustomEmojiOption): string | undefined {
  if (
    typeof emoji.width !== "number" ||
    !Number.isFinite(emoji.width) ||
    emoji.width <= 0 ||
    typeof emoji.height !== "number" ||
    !Number.isFinite(emoji.height) ||
    emoji.height <= 0
  ) {
    return undefined;
  }
  return `auto ${emoji.width} / ${emoji.height}`;
}

function CustomEmojiButton({
  emoji: e,
  imageUrl,
  onSelect,
}: {
  emoji: CustomEmojiOption;
  imageUrl: (url: string) => string;
  onSelect: (value: string) => void;
}) {
  const aspectRatio = customEmojiAspectRatio(e);
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            aria-label={`:${e.name}:`}
            onClick={() => onSelect(`:${e.name}:`)}
          />
        }
      >
        {e.name.startsWith("@") ? (
          <AccountGlyph name={e.name.slice(1)} />
        ) : (
          <img
            src={imageUrl(e.url)}
            alt={`:${e.name}:`}
            className="h-7 aspect-(--emoji-aspect-ratio) w-auto max-w-40 object-contain"
            style={
              aspectRatio ? ({ "--emoji-aspect-ratio": aspectRatio } as CSSProperties) : undefined
            }
          />
        )}
      </TooltipTrigger>
      <TooltipContent>{`:${e.name}:`}</TooltipContent>
    </Tooltip>
  );
}

function CustomEmojiGroups({
  search,
  onSelect,
}: {
  search: CustomEmojiSearch;
  onSelect: (value: string) => void;
}) {
  const { t } = useTranslation();
  const { filtered, imageUrl, searching } = search;
  // While searching, keep the ranked order (exact matches first): split into runs
  // of consecutive same-category emoji instead of regrouping by category, which
  // would let a weaker match jump ahead of a stronger one.
  const groups: { category: string; emojis: CustomEmojiOption[] }[] = [];
  if (searching) {
    for (const e of filtered) {
      const category = e.category ?? "";
      const last = groups[groups.length - 1];
      if (last?.category === category) last.emojis.push(e);
      else groups.push({ category, emojis: [e] });
    }
  } else {
    for (const category of new Set(filtered.map((e) => e.category ?? ""))) {
      groups.push({ category, emojis: filtered.filter((e) => (e.category ?? "") === category) });
    }
  }
  return (
    <>
      {groups.map(({ category, emojis }, index) => (
        <section key={`${index}:${category}`}>
          <h3 className="text-sm text-muted-foreground">
            {category || t("customEmoji.uncategorized")}
          </h3>
          <div className="flex flex-wrap gap-2">
            {emojis.map((e) => (
              <CustomEmojiButton key={e.name} emoji={e} imageUrl={imageUrl} onSelect={onSelect} />
            ))}
          </div>
        </section>
      ))}
    </>
  );
}

/** Custom emoji matches shown below the emoji grid while searching. */
export function CustomEmojiMatches({
  search,
  onSelect,
}: {
  search: CustomEmojiSearch;
  onSelect: (value: string) => void;
}) {
  const { t } = useTranslation();
  if (!search.filtered.length && !search.failed) return null;
  return (
    <div
      role="region"
      aria-label={t("home.customEmojis")}
      className="space-y-3 border-t border-border px-2 py-2"
    >
      {search.failed ? (
        <p role="alert">
          {t("home.controlsError")} <Button onClick={search.retry}>{t("home.refresh")}</Button>
        </p>
      ) : null}
      <CustomEmojiGroups search={search} onSelect={onSelect} />
    </div>
  );
}

export function CustomEmojiPicker({
  onSelect,
  search,
}: {
  onSelect: (value: string) => void;
  search: CustomEmojiSearch;
}) {
  const { t } = useTranslation();
  const { filtered, loading, failed, retry } = search;
  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {loading ? <p role="status">{t("home.loading")}</p> : null}
      {failed ? (
        <p role="alert">
          {t("home.controlsError")} <Button onClick={retry}>{t("home.refresh")}</Button>
        </p>
      ) : null}
      {!loading && !failed && !filtered.length ? <p>{t("home.noEmojis")}</p> : null}
      <div
        role="region"
        aria-label={t("home.customEmojis")}
        tabIndex={0}
        className="min-h-0 flex-1 space-y-3 overflow-y-auto overscroll-contain"
      >
        <CustomEmojiGroups search={search} onSelect={onSelect} />
      </div>
    </div>
  );
}
