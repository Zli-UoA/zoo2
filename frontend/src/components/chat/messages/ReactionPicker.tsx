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
          <DialogTitle>リアクションを追加</DialogTitle>
          <DialogDescription>
            絵文字を選ぶか、Emoji Kitchenで2つの絵文字を組み合わせます。
          </DialogDescription>
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
        <h3 className="font-medium">Emoji Kitchen</h3>
        {error ? (
          <div role="alert">
            絵文字の一覧を読み込めませんでした。
            <Button variant="outline" size="sm" onClick={retry}>
              再試行
            </Button>
          </div>
        ) : !catalog ? (
          <p role="status">絵文字を読み込み中…</p>
        ) : (
          <>
            <div className="flex items-end gap-2">
              {[
                { label: "1つ目の絵文字", value: left, set: setLeft },
                { label: "2つ目の絵文字", value: right, set: setRight },
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
                    alt="組み合わせのプレビュー"
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
                    <span role="status">プレビューを読み込み中…</span>
                  ) : !imageState.loaded ? (
                    <span role="alert">
                      画像を読み込めませんでした。別の組み合わせをお試しください。
                    </span>
                  ) : null}
                </>
              ) : (
                <>
                  <ReactionEmoji emoji={value} catalog={catalog} />
                  <span>この組み合わせは未対応です。</span>
                </>
              )}
            </div>
            <Button
              disabled={!url || imageState?.url !== url || !imageState.loaded}
              onClick={() => select(value)}
            >
              この組み合わせを追加
            </Button>
          </>
        )}
        <p className="text-xs text-muted-foreground">
          Emoji Kitchenの画像はGoogleから読み込みます。
        </p>
      </DialogContent>
    </Dialog>
  );
}
