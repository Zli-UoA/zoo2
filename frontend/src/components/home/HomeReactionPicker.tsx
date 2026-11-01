import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Tabs } from "@base-ui/react/tabs";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Spinner } from "../ui/spinner";
import { Dialog, DialogContent, DialogTitle } from "../ui/dialog";
import { EmojiPicker } from "../chat/messages/EmojiPicker";
import { ActivityPubEmoji, type CustomEmoji } from "./ActivityPubEmoji";
import { panelTabClassName } from "../ui/control-panel";
export interface ReactionChoice {
  reaction: string;
  emoji_id?: string;
}
export function HomeReactionPicker({
  postId,
  remote,
  onClose,
  onSelect,
}: {
  postId: string;
  remote?: boolean;
  onClose: () => void;
  onSelect: (reaction: ReactionChoice) => void;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [emojis, setEmojis] = useState<CustomEmoji[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [tab, setTab] = useState(remote ? "custom" : "unicode");
  const [search, setSearch] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(false);
    void client
      .get<{ emojis: CustomEmoji[] }>(
        `/api/activitypub/reaction-options?post_id=${encodeURIComponent(postId)}`,
        { signal: controller.signal },
      )
      .then((data) => {
        if (!controller.signal.aborted) setEmojis(data.emojis);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, postId, attempt]);
  const filtered = emojis.filter((e) => e.name.toLowerCase().includes(search.toLowerCase()));
  const select = (value: ReactionChoice) => {
    onSelect(value);
    onClose();
  };
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="flex h-[min(36rem,calc(100dvh-2rem))] flex-col overflow-hidden sm:max-w-md">
        <DialogTitle>{t("home.react")}</DialogTitle>
        <Tabs.Root
          className="flex min-h-0 flex-1 flex-col gap-3"
          value={tab}
          onValueChange={(value) => setTab(String(value))}
        >
          <Tabs.List aria-label={t("home.react")} className="flex gap-1 rounded-lg bg-muted p-1">
            <Tabs.Tab value="unicode" className={panelTabClassName}>
              {t("reactions.tabEmoji")}
            </Tabs.Tab>
            <Tabs.Tab value="custom" className={panelTabClassName}>
              {t("home.customEmojis")}
            </Tabs.Tab>
          </Tabs.List>
          <Tabs.Panel value="unicode" className="flex min-h-0 flex-1 flex-col">
            <div role="group" aria-label={t("home.quickReactions")} className="mb-2 flex gap-1">
              {["❤️", "👍", "🎉", "😊", "😢"].map((reaction) => (
                <Button
                  key={reaction}
                  variant="ghost"
                  size="icon-lg"
                  aria-label={reaction}
                  onClick={() => select({ reaction })}
                >
                  {reaction}
                </Button>
              ))}
            </div>
            <EmojiPicker onSelect={(reaction) => select({ reaction })} />
          </Tabs.Panel>
          <Tabs.Panel value="custom" className="flex min-h-0 flex-1 flex-col gap-3">
            <Input
              aria-label={t("home.searchEmoji")}
              placeholder={t("home.searchEmoji")}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            {loading ? (
              <Spinner label={t("home.loading")} />
            ) : error ? (
              <p role="alert" className="text-sm text-destructive">
                {t("home.controlsError")}{" "}
                <Button variant="link" onClick={() => setAttempt((n) => n + 1)}>
                  {t("home.refresh")}
                </Button>
              </p>
            ) : (
              <>
                {filtered.length === 0 ? (
                  <p className="text-sm text-muted-foreground">{t("home.noEmojis")}</p>
                ) : null}
                <div className="grid min-h-0 grid-cols-6 content-start gap-1 overflow-y-auto">
                  {filtered.slice(0, 200).map((e) => (
                    <Button
                      key={`${e.name}:${e.image_id}`}
                      variant="ghost"
                      size="icon-lg"
                      aria-label={e.name}
                      onClick={() => select({ reaction: e.name, emoji_id: e.image_id })}
                    >
                      <ActivityPubEmoji emoji={e} className="size-7 object-contain" />
                    </Button>
                  ))}
                </div>
                {filtered.length > 200 ? (
                  <p className="text-xs text-muted-foreground">{t("home.emojiSearchMore")}</p>
                ) : null}
              </>
            )}
          </Tabs.Panel>
        </Tabs.Root>
      </DialogContent>
    </Dialog>
  );
}
