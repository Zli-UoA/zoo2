import { useTranslation } from "react-i18next";
import { ArrowDownIcon, ArrowUpIcon, MoreHorizontalIcon, PinIcon, PinOffIcon } from "lucide-react";

import { usePinnedChannels } from "../../hooks/usePinnedChannels";
import { Button } from "../ui/button";
import { cn } from "cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";

/**
 * チャンネル・DM のピン留めメニュー。ピン中は解除と並び替えを出す。
 * 並び替えは手元の順序を受けて `reorderPins` に渡す。
 */
export function ChannelPinMenu({
  channelId,
  itemLabel,
  isDm = false,
  onMoveUp,
  onMoveDown,
  canMoveUp = false,
  canMoveDown = false,
  className,
}: {
  channelId: string;
  /** 対象のチャンネル名・DM 名。読み上げで、どの行のメニューかを区別する。 */
  itemLabel?: string;
  isDm?: boolean;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  canMoveUp?: boolean;
  canMoveDown?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const pins = usePinnedChannels();
  const pinned = pins.isPinned(channelId);
  const pending = pins.pending.has(channelId);
  if (!pins.available) {
    return null;
  }
  const label = itemLabel
    ? t(isDm ? "channelPins.dmMenu" : "channelPins.channelMenu", { name: itemLabel })
    : t("channelPins.menu");

  return (
    <DropdownMenu>
      {/* 書き込み中もトリガーは無効にしない。ネイティブの `disabled` を付けると、
          メニューを閉じたときにフォーカスを戻せず `document.body` へ落ちる。二重操作は項目側で止める。 */}
      <span className={cn("inline-flex", pending && "opacity-50", className)}>
        <DropdownMenuTrigger
          aria-label={label}
          title={label}
          render={
            <Button type="button" variant="ghost" size="icon-sm" className="size-9 md:size-7" />
          }
        >
          <MoreHorizontalIcon />
        </DropdownMenuTrigger>
      </span>
      <DropdownMenuContent align="end">
        <DropdownMenuItem
          className="min-h-11"
          disabled={pending}
          onClick={() => {
            void (pinned ? pins.unpinChannel(channelId) : pins.pinChannel(channelId));
          }}
        >
          {pinned ? <PinOffIcon /> : <PinIcon />}
          {t(pinned ? "channelPins.unpin" : "channelPins.pin")}
        </DropdownMenuItem>
        {pinned && (onMoveUp || onMoveDown) ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="min-h-11"
              disabled={pending || !canMoveUp}
              onClick={onMoveUp}
            >
              <ArrowUpIcon />
              {t("channelPins.moveUp")}
            </DropdownMenuItem>
            <DropdownMenuItem
              className="min-h-11"
              disabled={pending || !canMoveDown}
              onClick={onMoveDown}
            >
              <ArrowDownIcon />
              {t("channelPins.moveDown")}
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
