import { useTranslation } from "react-i18next";
import { useState } from "react";

import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { kitchenImageUrl, kitchenValue } from "../../../lib/emoji/kitchen";
import { Button } from "../../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { ReactionEmoji } from "./ReactionEmoji";

const QUICK_EMOJI = ["👍", "❤️", "😂", "🎉", "👀", "🙏"];

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
  const value = kitchenValue(left, right);
  const url = catalog ? kitchenImageUrl(catalog, value) : null;
  const select = (emoji: string) => {
    onSelect(emoji);
    onOpenChange(false);
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("reactions.add")}</DialogTitle>
          <DialogDescription>{t("reactions.kitchenDescription")}</DialogDescription>
        </DialogHeader>
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
        {error ? (
          <div role="alert">
            {t("reactions.kitchenLoadError")}
            <Button variant="outline" size="sm" onClick={retry}>
              {t("reactions.retry")}
            </Button>
          </div>
        ) : !catalog ? (
          <p role="status">{t("reactions.kitchenLoading")}</p>
        ) : (
          <>
            <div className="flex items-end gap-2">
              {[
                { label: t("reactions.firstEmoji"), value: left, set: setLeft },
                { label: t("reactions.secondEmoji"), value: right, set: setRight },
              ].map((field) => (
                <label key={field.label} className="flex min-w-0 flex-1 flex-col gap-1 text-xs">
                  {field.label}
                  <select
                    className="w-full rounded-lg border bg-background p-2 text-base"
                    value={field.value}
                    onChange={(event) => field.set(event.target.value)}
                  >
                    {catalog.emojis.map(([codepoint, emoji, name]) => (
                      <option key={codepoint} value={codepoint}>
                        {emoji} {name}
                      </option>
                    ))}
                  </select>
                </label>
              ))}
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
      </DialogContent>
    </Dialog>
  );
}
