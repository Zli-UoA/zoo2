import { Combobox } from "@base-ui/react/combobox";
import { SearchIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useMemo, useState } from "react";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import {
  kitchenEmojis,
  kitchenImageUrl,
  kitchenValue,
  matchesKitchenQuery,
} from "../../../lib/emoji/kitchen";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { inputClassName } from "../../ui/input-styles";
import { ReactionEmoji } from "./ReactionEmoji";

const QUICK_EMOJI = [
  "👍",
  "❤️",
  "😄",
  "🎉",
  "🙌",
  "🔥",
  "👀",
  "🚀",
  "✅",
  "🙏",
  "💯",
  "🤔",
  "😂",
  "🥳",
  "👏",
  "💪",
  "😢",
  "😮",
  "🐛",
  "☕",
  "🍕",
  "🌱",
  "⚡",
  "✨",
];

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

export function ReactionPicker({
  open,
  onOpenChange,
  onSelect,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelect: (emoji: string) => void;
}) {
  const { t } = useTranslation();
  const { catalog, error, retry } = useKitchenCatalog(open);
  const [left, setLeft] = useState("1f600");
  const [right, setRight] = useState("1f431");
  const [imageState, setImageState] = useState<{ url: string; loaded: boolean } | null>(null);
  const [dialogElement, setDialogElement] = useState<HTMLDivElement | null>(null);
  const value = kitchenValue(left, right);
  const url = catalog ? kitchenImageUrl(catalog, value) : null;
  const select = (emoji: string) => {
    onSelect(emoji);
    onOpenChange(false);
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        ref={setDialogElement}
        className="flex max-h-[calc(100dvh-2rem)] flex-col gap-4 overflow-hidden"
      >
        <DialogHeader className="shrink-0 pr-8">
          <DialogTitle>{t("reactions.add")}</DialogTitle>
          <DialogDescription>{t("reactions.kitchenDescription")}</DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
          <div className="flex flex-wrap gap-2">
            {QUICK_EMOJI.map((emoji) => (
              <Button
                key={emoji}
                variant="outline"
                size="icon"
                aria-label={emoji}
                onClick={() => select(emoji)}
              >
                {emoji}
              </Button>
            ))}
          </div>
          <h3 className="font-medium">{t("reactions.kitchen")}</h3>
          {!catalog ? (
            <div className="flex flex-col gap-4">
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
                className="flex min-h-28 flex-col items-center justify-center gap-2"
              >
                <p className={error ? "text-destructive" : undefined}>
                  {error ? t("reactions.kitchenLoadError") : t("reactions.kitchenLoading")}
                </p>
                {error ? (
                  <Button variant="outline" size="sm" onClick={retry}>
                    {t("reactions.retry")}
                  </Button>
                ) : null}
              </div>
              <Button disabled>{t("reactions.addCombination")}</Button>
            </div>
          ) : (
            <>
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
                className="flex min-h-28 flex-col items-center justify-center gap-2"
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
                      className={
                        imageState?.url === url && !imageState.loaded ? "hidden" : "object-contain"
                      }
                      onLoad={() => setImageState({ url, loaded: true })}
                      onError={() => setImageState({ url, loaded: false })}
                    />
                    {imageState?.url !== url ? (
                      <span role="status">{t("reactions.previewLoading")}</span>
                    ) : !imageState.loaded ? (
                      <span role="alert">{t("reactions.previewError")}</span>
                    ) : null}
                  </>
                ) : (
                  <>
                    <ReactionEmoji emoji={value} catalog={catalog} />
                    <span>{t("reactions.unsupported")}</span>
                  </>
                )}
              </div>
              <Button
                disabled={!url || imageState?.url !== url || !imageState.loaded}
                onClick={() => select(value)}
              >
                {t("reactions.addCombination")}
              </Button>
            </>
          )}
          <p className="text-xs text-muted-foreground">{t("reactions.kitchenImageNotice")}</p>
        </div>
      </DialogContent>
    </Dialog>
  );
}
