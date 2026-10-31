import { noAutofill } from "../../ui/autofill";
import { Combobox } from "@base-ui/react/combobox";
import { SearchIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import {
  kitchenEmojis,
  kitchenImageUrl,
  kitchenValue,
  matchesKitchenQuery,
} from "../../../lib/emoji/kitchen";
import { Button } from "../../ui/button";
import { inputClassName } from "../../ui/input-styles";
import { Skeleton } from "../../ui/skeleton";
import { ReactionEmoji } from "./ReactionEmoji";

function KitchenEmojiCombobox({
  label,
  catalog,
  value,
  onChange,
  dialogElement,
}: {
  label: string;
  catalog: KitchenCatalog;
  value: string;
  onChange: (codepoint: string) => void;
  dialogElement: HTMLDivElement | null;
}) {
  const { t } = useTranslation();
  const emojis = kitchenEmojis(catalog);
  const options = useMemo(() => emojis.map((emoji) => emoji.codepoint), [emojis]);
  const byCodepoint = useMemo(
    () => new Map(emojis.map((emoji) => [emoji.codepoint, emoji])),
    [emojis],
  );
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1 text-xs">
      <span>{label}</span>
      <Combobox.Root
        items={options}
        value={value}
        onValueChange={(next) => {
          if (next) onChange(next);
        }}
        itemToStringLabel={(codepoint) => {
          const emoji = byCodepoint.get(codepoint);
          return emoji ? `${emoji.emoji} ${emoji.name.replaceAll("_", " ")}` : codepoint;
        }}
        filter={(codepoint, query) => {
          const emoji = byCodepoint.get(codepoint);
          return emoji ? matchesKitchenQuery(emoji, query) : true;
        }}
        openOnInputClick
      >
        <div className="relative">
          <SearchIcon
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
          />
          <Combobox.Input
            {...noAutofill}
            className={`${inputClassName()} w-full pl-8`}
            placeholder={t("reactions.searchEmoji")}
            aria-label={label}
          />
        </div>
        <Combobox.Portal container={dialogElement}>
          <Combobox.Positioner
            side="bottom"
            align="start"
            sideOffset={4}
            collisionBoundary={dialogElement ?? undefined}
            collisionPadding={8}
            collisionAvoidance={{ side: "none", align: "shift", fallbackAxisSide: "none" }}
            className="z-20"
          >
            <Combobox.Popup className="flex max-h-[min(12rem,var(--available-height))] w-[max(var(--anchor-width),14rem)] max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-md">
              <Combobox.List aria-label={label} className="min-h-0 flex-1 overflow-y-auto">
                {(codepoint: string) => {
                  const emoji = byCodepoint.get(codepoint);
                  return emoji ? (
                    <Combobox.Item
                      key={emoji.codepoint}
                      value={emoji.codepoint}
                      className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground data-[selected]:font-medium"
                    >
                      <span aria-hidden="true" className="text-base leading-none">
                        {emoji.emoji}
                      </span>
                      <span className="truncate">{emoji.name.replaceAll("_", " ")}</span>
                    </Combobox.Item>
                  ) : null;
                }}
              </Combobox.List>
              <Combobox.Empty className="px-2 py-1.5 text-xs text-muted-foreground">
                {t("reactions.noSearchResults")}
              </Combobox.Empty>
            </Combobox.Popup>
          </Combobox.Positioner>
        </Combobox.Portal>
      </Combobox.Root>
    </div>
  );
}

export function EmojiKitchenPicker({
  active,
  onSelect,
  dialogElement,
}: {
  active: boolean;
  onSelect: (emoji: string) => void;
  dialogElement: HTMLDivElement | null;
}) {
  const { t } = useTranslation();
  const { catalog, error, retry } = useKitchenCatalog(active);
  const [left, setLeft] = useState("1f600");
  const [right, setRight] = useState("1f431");
  const [imageState, setImageState] = useState<{ url: string; loaded: boolean } | null>(null);
  const value = kitchenValue(left, right);
  const url = catalog ? kitchenImageUrl(catalog, value) : null;

  if (!catalog) {
    return (
      <div className="flex h-full min-h-0 flex-col gap-3">
        <div className="flex items-start gap-2">
          {[t("reactions.firstEmoji"), t("reactions.secondEmoji")].map((label) => (
            <div key={label} className="flex min-w-0 flex-1 flex-col gap-1 text-xs">
              <span>{label}</span>
              {error ? (
                <div
                  aria-hidden="true"
                  className="h-8 w-full rounded-lg border border-dashed bg-muted/30"
                />
              ) : (
                <Skeleton className="h-8 w-full rounded-lg" />
              )}
            </div>
          ))}
        </div>
        <div
          role={error ? "alert" : "status"}
          className="flex h-40 shrink-0 flex-col items-center justify-center gap-2 rounded-lg border border-border"
        >
          <p className={error ? "text-destructive" : "text-sm"}>
            {error ? t("reactions.kitchenLoadError") : t("reactions.kitchenLoading")}
          </p>
          {error ? (
            <Button variant="outline" size="sm" onClick={retry}>
              {t("reactions.retry")}
            </Button>
          ) : null}
        </div>
        <Button disabled>{t("reactions.addCombination")}</Button>
        <p className="text-xs text-muted-foreground">{t("reactions.kitchenImageNotice")}</p>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-y-auto">
      <div className="flex items-start gap-2">
        <KitchenEmojiCombobox
          label={t("reactions.firstEmoji")}
          catalog={catalog}
          value={left}
          onChange={setLeft}
          dialogElement={dialogElement}
        />
        <KitchenEmojiCombobox
          label={t("reactions.secondEmoji")}
          catalog={catalog}
          value={right}
          onChange={setRight}
          dialogElement={dialogElement}
        />
      </div>
      <div
        className="flex h-40 shrink-0 flex-col items-center justify-center gap-2 rounded-lg border border-border"
        aria-live="polite"
      >
        {url ? (
          <>
            <img
              key={url}
              src={url}
              alt={t("reactions.previewAlt")}
              width={80}
              height={80}
              referrerPolicy="no-referrer"
              className={cn(
                "size-20 object-contain",
                imageState?.url === url && !imageState.loaded && "invisible",
              )}
              onLoad={() => setImageState({ url, loaded: true })}
              onError={() => setImageState({ url, loaded: false })}
            />
            {imageState?.url !== url ? (
              <span role="status" className="text-xs text-muted-foreground">
                {t("reactions.previewLoading")}
              </span>
            ) : !imageState.loaded ? (
              <span role="alert" className="text-xs text-destructive">
                {t("reactions.previewError")}
              </span>
            ) : null}
          </>
        ) : (
          <>
            <ReactionEmoji emoji={value} catalog={catalog} size={64} />
            <span className="text-xs text-muted-foreground">{t("reactions.unsupported")}</span>
          </>
        )}
      </div>
      <Button
        disabled={!url || imageState?.url !== url || !imageState.loaded}
        onClick={() => onSelect(value)}
      >
        {t("reactions.addCombination")}
      </Button>
      <p className="text-xs text-muted-foreground">{t("reactions.kitchenImageNotice")}</p>
    </div>
  );
}
