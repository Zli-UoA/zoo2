import { cn } from "cn";

/**
 * ドラッグ中の挿入位置を示す線。置き先の要素（`relative`）の上端（`before`）か下端（`after`）に出す。
 * ピン留めの行（`ChannelListItem`・`DmListItem`）と、行とスレッドを囲むブロック（`PinnedSection`）で共有する。
 */
export function DropIndicator({ edge }: { edge: "before" | "after" }) {
  return (
    <span
      aria-hidden="true"
      data-drop-indicator={edge}
      className={cn(
        "pointer-events-none absolute inset-x-0 z-20 h-[2px] rounded-full bg-foreground",
        edge === "before" ? "-top-px" : "-bottom-px",
      )}
    />
  );
}
