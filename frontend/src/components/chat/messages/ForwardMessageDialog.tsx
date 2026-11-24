import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "cn";
import {
  CheckIcon,
  HashIcon,
  LinkIcon,
  LockKeyholeIcon,
  MessageCircleIcon,
  SearchIcon,
  XIcon,
} from "lucide-react";

import { canPostInChannel } from "../../../hooks/channelPermissions";
import type { ResolvedForwardMessage } from "../../../lib/api/types";
import type { ChannelRow, ChannelUserRow, UserRow } from "../../../lib/electric/types";
import type { Locale } from "../../../lib/i18n/locale";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";
import { buildDmLabels } from "../dm";
import { detectComposerPlatform, shouldSendFromKey } from "./Composer";
import { ForwardedMessageCard } from "./ForwardedMessageCard";
import { messageLinkUrl } from "./messageLink";

export interface ForwardDestination {
  id: string;
  label: string;
  isDm: boolean;
  isPrivate: boolean;
}

export function buildForwardDestinations({
  channels,
  memberships,
  users,
  currentUserId,
  isAdmin,
  locale,
  t,
}: {
  channels: readonly ChannelRow[];
  memberships: readonly ChannelUserRow[];
  users: readonly Pick<UserRow, "id" | "user_name" | "display_name">[];
  currentUserId: string | null;
  isAdmin: boolean;
  locale: Locale;
  t: TFunction;
}): ForwardDestination[] {
  if (!currentUserId) return [];
  const joinedIds = new Set(
    memberships
      .filter((member) => member.user_id === currentUserId)
      .map((member) => member.channel_id),
  );
  const eligible = channels.filter((channel) => {
    if (!joinedIds.has(channel.id)) return false;
    if (channel.deleted_at || channel.archive_at) return false;
    if (channel.encrypted || channel.channel_name.startsWith("edm:")) return false;
    return canPostInChannel({
      postingPolicy: channel.posting_policy,
      isAdmin,
      isCreator: channel.created_user_id === currentUserId,
    });
  });

  const dmLabels = buildDmLabels({
    channels: eligible,
    memberships,
    users,
    currentUserId,
    locale,
    t,
  });

  return eligible
    .map((channel) => ({
      id: channel.id,
      label: channel.is_dm
        ? (dmLabels.get(channel.id) ?? t("catchUp.conversation"))
        : channel.channel_name,
      isDm: channel.is_dm,
      isPrivate: channel.is_private,
    }))
    .sort((a, b) => {
      if (a.isDm !== b.isDm) return a.isDm ? 1 : -1;
      return a.label.localeCompare(b.label, locale);
    });
}

export interface ForwardMessageDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  source: ResolvedForwardMessage | null;
  destinations: readonly ForwardDestination[];
  isSubmitting?: boolean;
  error?: string | null;
  defaultSelectedId?: string | null;
  defaultComment?: string;
  onSubmit: (destinationChannelId: string, comment: string) => void;
}

/**
 * メッセージを他の参加中平文チャンネル・DM へ転送（引用投稿）するモーダルダイアログ。
 * 転送先 1 件選択＋任意コメント（空可）＋元投稿プレビュー＋リンクコピーを提供する。
 */
export function ForwardMessageDialog({
  open,
  onOpenChange,
  source,
  destinations,
  isSubmitting = false,
  error = null,
  defaultSelectedId = null,
  defaultComment = "",
  onSubmit,
}: ForwardMessageDialogProps) {
  const { t } = useTranslation();
  const [selectedId, setSelectedId] = useState<string | null>(defaultSelectedId);
  const [search, setSearch] = useState("");
  const [comment, setComment] = useState(defaultComment);
  const [activeIndex, setActiveIndex] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const listboxId = useId();
  const [platform] = useState(() => detectComposerPlatform());

  useEffect(() => {
    if (open) {
      setSelectedId(defaultSelectedId);
      setSearch("");
      setComment(defaultComment);
      setActiveIndex(0);
    }
  }, [open, source?.id, defaultSelectedId, defaultComment]);

  const selectedDestination = useMemo(
    () => destinations.find((d) => d.id === selectedId) ?? null,
    [destinations, selectedId],
  );

  const filteredDestinations = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return destinations;
    return destinations.filter((d) => d.label.toLowerCase().includes(q));
  }, [destinations, search]);

  useEffect(() => {
    setActiveIndex(0);
  }, [search, destinations.length]);

  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const sourceLink =
    source && origin
      ? messageLinkUrl(origin, source.channel_id, source.id, source.parent_id)
      : null;

  async function handleCopyLink() {
    if (!sourceLink) return;
    try {
      await navigator.clipboard.writeText(sourceLink);
      toast.success(t("messages.linkCopied"));
    } catch {
      toast.error(t("messages.linkCopyFailed"));
    }
  }

  function handleSelectDestination(destId: string) {
    setSelectedId(destId);
    setSearch("");
  }

  function handleSubmit() {
    if (!selectedDestination || isSubmitting) return;
    onSubmit(selectedDestination.id, comment.trim());
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[calc(100dvh-2rem)] flex-col gap-3 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("messages.forwardTitle")}</DialogTitle>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-0.5">
          <div className="flex flex-col gap-1.5">
            {selectedDestination ? (
              <div className="flex min-h-9 items-center justify-between gap-2 rounded-lg border bg-background px-2.5 py-1.5">
                <span className="inline-flex min-w-0 items-center gap-1 rounded-md bg-brand-muted/60 px-2 py-0.5 text-sm font-medium text-foreground">
                  {selectedDestination.isDm ? (
                    <MessageCircleIcon className="size-3.5 shrink-0" aria-hidden="true" />
                  ) : selectedDestination.isPrivate ? (
                    <LockKeyholeIcon className="size-3.5 shrink-0" aria-hidden="true" />
                  ) : (
                    <HashIcon className="size-3.5 shrink-0" aria-hidden="true" />
                  )}
                  <span className="truncate">{selectedDestination.label}</span>
                  <button
                    type="button"
                    className="ml-1 inline-flex size-4 items-center justify-center rounded-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
                    aria-label={t("messages.forwardClearDestination", {
                      name: selectedDestination.label,
                    })}
                    onClick={() => {
                      setSelectedId(null);
                      requestAnimationFrame(() => searchInputRef.current?.focus());
                    }}
                  >
                    <XIcon className="size-3" aria-hidden="true" />
                  </button>
                </span>
              </div>
            ) : (
              <div className="flex flex-col gap-1.5">
                <div className="relative">
                  <SearchIcon
                    className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <Input
                    ref={searchInputRef}
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                    placeholder={t("messages.forwardSearchPlaceholder")}
                    aria-label={t("messages.forwardDestinationLabel")}
                    role="combobox"
                    aria-expanded={filteredDestinations.length > 0}
                    aria-autocomplete="list"
                    aria-controls={filteredDestinations.length > 0 ? listboxId : undefined}
                    aria-activedescendant={
                      filteredDestinations[activeIndex]
                        ? `${listboxId}-opt-${activeIndex}`
                        : undefined
                    }
                    className="pl-8"
                    onKeyDown={(event) => {
                      if (filteredDestinations.length === 0) return;
                      if (event.key === "ArrowDown") {
                        event.preventDefault();
                        setActiveIndex((prev) => (prev + 1) % filteredDestinations.length);
                      } else if (event.key === "ArrowUp") {
                        event.preventDefault();
                        setActiveIndex(
                          (prev) =>
                            (prev - 1 + filteredDestinations.length) % filteredDestinations.length,
                        );
                      } else if (event.key === "Enter") {
                        event.preventDefault();
                        const chosen = filteredDestinations[activeIndex];
                        if (chosen) handleSelectDestination(chosen.id);
                      }
                    }}
                  />
                </div>
                {destinations.length === 0 ? (
                  <p className="rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
                    {t("messages.forwardNoDestinations")}
                  </p>
                ) : filteredDestinations.length === 0 ? (
                  <p className="rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
                    {t("channels.searchEmpty")}
                  </p>
                ) : (
                  <div
                    id={listboxId}
                    role="listbox"
                    aria-label={t("messages.forwardDestinationLabel")}
                    className="max-h-40 overflow-y-auto rounded-lg border bg-popover p-1 shadow-xs"
                  >
                    {filteredDestinations.map((dest, index) => (
                      <button
                        key={dest.id}
                        id={`${listboxId}-opt-${index}`}
                        type="button"
                        role="option"
                        aria-selected={index === activeIndex}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors outline-none",
                          index === activeIndex
                            ? "bg-accent text-accent-foreground"
                            : "hover:bg-muted/70",
                        )}
                        onMouseEnter={() => setActiveIndex(index)}
                        onClick={() => handleSelectDestination(dest.id)}
                      >
                        {dest.isDm ? (
                          <MessageCircleIcon
                            className="size-3.5 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                        ) : dest.isPrivate ? (
                          <LockKeyholeIcon
                            className="size-3.5 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                        ) : (
                          <HashIcon
                            className="size-3.5 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                        )}
                        <span className="min-w-0 flex-1 truncate">{dest.label}</span>
                        {dest.id === selectedId ? (
                          <CheckIcon className="size-3.5 shrink-0" aria-hidden="true" />
                        ) : null}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>

          <Textarea
            value={comment}
            onChange={(event) => setComment(event.target.value)}
            rows={3}
            placeholder={t("messages.forwardCommentPlaceholder")}
            aria-label={t("messages.forwardCommentLabel")}
            className="min-h-20 resize-y text-base md:text-sm"
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
              if (
                shouldSendFromKey(
                  {
                    key: event.key,
                    metaKey: event.metaKey,
                    ctrlKey: event.ctrlKey,
                    shiftKey: event.shiftKey,
                    isComposing: event.nativeEvent.isComposing,
                  },
                  platform,
                )
              ) {
                event.preventDefault();
                handleSubmit();
              }
            }}
          />

          {source ? (
            <div aria-label={t("messages.forwardPreviewLabel")}>
              <ForwardedMessageCard
                messageId={source.id}
                entry={{ status: "ok", ...source }}
                compact
              />
            </div>
          ) : null}

          {error ? (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          ) : null}
        </div>

        <DialogFooter className="sm:justify-between">
          <Button
            type="button"
            variant="outline"
            disabled={!sourceLink}
            onClick={() => void handleCopyLink()}
          >
            <LinkIcon className="size-4" aria-hidden="true" />
            {t("messages.copyLink")}
          </Button>
          <Button
            type="button"
            disabled={!selectedDestination || isSubmitting}
            aria-busy={isSubmitting}
            onClick={handleSubmit}
          >
            {isSubmitting ? t("messages.forwarding") : t("messages.forwardSubmit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
