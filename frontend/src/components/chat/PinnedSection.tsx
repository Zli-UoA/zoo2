import { GripVerticalIcon, PinIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import { moveWithin, usePinnedChannels } from "../../hooks/usePinnedChannels";
import type { ChannelRow, ChannelUserRow, UserRow } from "../../lib/electric/types";
import { useLocale } from "../i18n/LocaleProvider";
import { Button } from "../ui/button";
import { CollapsibleSection } from "../ui/collapsible-section";
import { ChannelListItem } from "./ChannelListItem";
import { ChannelPinMenu } from "./ChannelPinMenu";
import { buildDmLabels, parseOneToOneDmIds } from "./dm";
import { DmListItem } from "./DmListItem";

/**
 * サイドバー先頭の「ピン留め」。通常チャンネルと DM を、ユーザーごとの順で出す。
 * 並び替えはドラッグ、メニュー、`Alt+↑` / `Alt+↓` のどれでもできる。ほかの書き込みが
 * 終わるまでは、並びを変える操作を止める。
 */
export function PinnedSection({
  channels,
  memberships,
  users,
  joinedChannelIds,
  currentUserId,
  activeChannelId,
  unreadChannelIds,
  onOpen,
  loading,
  className,
}: {
  channels: ChannelRow[];
  memberships: ChannelUserRow[];
  users: UserRow[];
  joinedChannelIds: ReadonlySet<string>;
  currentUserId: string | null;
  activeChannelId: string | null;
  unreadChannelIds: ReadonlySet<string>;
  onOpen: () => void;
  loading: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const pins = usePinnedChannels();
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const busy = pins.pending.size > 0;

  const channelById = useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );
  // アーカイブ中は表示しない（ピンは残るので、アンアーカイブで元の位置に戻る）。
  const pinnedChannels = useMemo(
    () =>
      pins.pins
        .map((pin) => channelById.get(pin.channel_id))
        .filter((channel): channel is ChannelRow => Boolean(channel))
        .filter((channel) => !channel.deleted_at && !channel.archive_at)
        .filter((channel) => !channel.is_dm || joinedChannelIds.has(channel.id)),
    [channelById, joinedChannelIds, pins.pins],
  );
  const dmLabels = useMemo(
    () =>
      buildDmLabels({
        channels: pinnedChannels.filter((channel) => channel.is_dm),
        memberships,
        users,
        currentUserId,
        locale,
        t,
      }),
    [currentUserId, locale, memberships, pinnedChannels, t, users],
  );

  const ids = useMemo(() => pinnedChannels.map((channel) => channel.id), [pinnedChannels]);

  // ピンを解除して行が消えるとき、フォーカスを通常の一覧へ移す。
  // 解除した行は同じチャンネル/DM の一覧にもあるため、そちらのリンクを優先する。
  const pinnedSectionRef = useRef<HTMLDivElement | null>(null);
  // 同時に複数のピンが消えたとき、直前にメニューを開いていた行を選ぶための記録。
  const lastFocusedPinId = useRef<string | null>(null);
  const previousIds = useRef<string[]>([]);
  useEffect(() => {
    const previous = previousIds.current;
    const current = pinnedChannels.map((channel) => channel.id);
    previousIds.current = current;
    const currentSet = new Set(current);
    const removed = previous.filter((id) => !currentSet.has(id));
    if (removed.length === 0) {
      return;
    }
    const focusedPinId = lastFocusedPinId.current;
    const target =
      focusedPinId !== null && removed.includes(focusedPinId) ? focusedPinId : removed[0]!;
    const frame = requestAnimationFrame(() => {
      const active = document.activeElement;
      // ほかの操作（ヘッダーや通常一覧の行メニュー）がフォーカスを持っているときは奪わない。
      if (active && active !== document.body && active.isConnected) {
        return;
      }
      // チャンネル ID をセレクターへ埋め込むとエスケープが要るため、`href` を直接比べる。
      const link = Array.from(document.querySelectorAll<HTMLElement>("aside a")).find(
        (anchor) => anchor.getAttribute("href") === `/channels/${target}`,
      );
      if (link) {
        link.focus();
        // 折りたたまれた一覧（`<div hidden>`）の中のリンクはフォーカスを受け取れない。
        if (document.activeElement === link) {
          return;
        }
        // 節を開き直せるよう、フォーカスはその節の見出しへ移す。
        const sectionHeading = link.closest("section")?.querySelector<HTMLElement>("h2 button");
        sectionHeading?.focus();
        if (sectionHeading && document.activeElement === sectionHeading) {
          return;
        }
      }
      // 一覧に同じ項目が無いとき（未参加の公開チャンネルなど）は、ピンが残っていれば
      // ピン留めの節の見出しへ、全部消えたらサイドバー先頭の節の見出しへ移す。
      const fallback =
        current.length > 0
          ? pinnedSectionRef.current?.querySelector<HTMLElement>("h2 button")
          : document.querySelector<HTMLElement>("aside h2 button");
      fallback?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [pinnedChannels]);

  function reorderTo(channelId: string, targetIndex: number) {
    if (busy) {
      return;
    }
    void pins.reorderPins(moveWithin(ids, channelId, targetIndex), channelId);
  }

  function handleKeyDown(channelId: string, event: KeyboardEvent<HTMLDivElement>) {
    if (busy || !event.altKey) {
      return;
    }
    const index = ids.indexOf(channelId);
    if (event.key === "ArrowUp" && index > 0) {
      event.preventDefault();
      reorderTo(channelId, index - 1);
    } else if (event.key === "ArrowDown" && index >= 0 && index < ids.length - 1) {
      event.preventDefault();
      reorderTo(channelId, index + 1);
    }
  }

  function dragHandlers(channelId: string) {
    return {
      draggable: !busy,
      onDragStart: () => setDraggingId(channelId),
      onDragEnd: () => setDraggingId(null),
      onDragOver: (event: DragEvent<HTMLDivElement>) => {
        if (busy) {
          return;
        }
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
      },
      onDrop: () => {
        if (busy) {
          return;
        }
        const target = ids.indexOf(channelId);
        if (draggingId && draggingId !== channelId && target >= 0) {
          reorderTo(draggingId, target);
        }
        setDraggingId(null);
      },
    };
  }

  if (!pins.available) {
    return null;
  }
  if (loading && pinnedChannels.length === 0 && !pins.error) {
    return null;
  }
  if (pinnedChannels.length === 0 && !pins.error) {
    return null;
  }

  return (
    <div ref={pinnedSectionRef} className="contents">
      <CollapsibleSection
        title={t("channelPins.title")}
        icon={<PinIcon />}
        count={pinnedChannels.length}
        className={className}
      >
        <div className="flex flex-col gap-0.5 px-2 pb-2">
          {pins.error ? (
            <div
              role="alert"
              className="mx-1 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive"
            >
              <span className="min-w-0 flex-1">{pins.error}</span>
              <Button
                variant="ghost"
                size="icon-sm"
                className="-my-1 size-6 shrink-0"
                aria-label={t("channelPins.dismissError")}
                title={t("channelPins.dismissError")}
                onClick={pins.dismissError}
              >
                ×
              </Button>
            </div>
          ) : null}
          {pinnedChannels.map((channel, index) => {
            const active = activeChannelId === channel.id;
            const unread = unreadChannelIds.has(channel.id);
            const label = channel.is_dm
              ? (dmLabels.get(channel.id) ?? t("dms.untitled"))
              : channel.channel_name;
            const menu = (
              <ChannelPinMenu
                channelId={channel.id}
                itemLabel={label}
                isDm={channel.is_dm}
                canMoveUp={!busy && index > 0}
                canMoveDown={!busy && index < pinnedChannels.length - 1}
                onMoveUp={() => reorderTo(channel.id, index - 1)}
                onMoveDown={() => reorderTo(channel.id, index + 1)}
              />
            );
            const dragHandle = (
              <span
                aria-hidden="true"
                title={t("channelPins.dragHandle")}
                className="hidden cursor-grab items-center text-muted-foreground md:flex"
              >
                <GripVerticalIcon className="size-3.5" />
              </span>
            );
            const actions = (
              <div
                className="flex items-center gap-0.5"
                onFocusCapture={() => {
                  lastFocusedPinId.current = channel.id;
                }}
              >
                {dragHandle}
                {menu}
              </div>
            );
            const dragProps = dragHandlers(channel.id);
            if (channel.is_dm) {
              const dmIds = parseOneToOneDmIds(channel.channel_name);
              const peerId =
                currentUserId && dmIds?.includes(currentUserId)
                  ? dmIds.find((id) => id !== currentUserId)
                  : undefined;
              return (
                <DmListItem
                  key={channel.id}
                  channel={channel}
                  label={label}
                  avatarUrl={users.find((user) => user.id === peerId)?.avatar_url}
                  active={active}
                  unread={unread}
                  onOpen={onOpen}
                  actions={actions}
                  onKeyDown={(event) => handleKeyDown(channel.id, event)}
                  draggable={dragProps.draggable}
                  dragging={draggingId === channel.id}
                  onDragStart={() => setDraggingId(channel.id)}
                  onDragEnd={dragProps.onDragEnd}
                  onDragOver={dragProps.onDragOver}
                  onDrop={dragProps.onDrop}
                />
              );
            }
            return (
              <ChannelListItem
                key={channel.id}
                channel={channel}
                active={active}
                unread={unread}
                onOpen={onOpen}
                actions={actions}
                onKeyDown={(event) => handleKeyDown(channel.id, event)}
                draggable={dragProps.draggable}
                dragging={draggingId === channel.id}
                onDragStart={dragProps.onDragStart}
                onDragEnd={dragProps.onDragEnd}
                onDragOver={dragProps.onDragOver}
                onDrop={dragProps.onDrop}
              />
            );
          })}
        </div>
      </CollapsibleSection>
    </div>
  );
}
