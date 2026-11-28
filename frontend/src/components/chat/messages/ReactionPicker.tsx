import { kitchenShortcode } from "../../../lib/emoji/kitchen";
import { Tabs } from "@base-ui/react/tabs";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { EmojiKitchenPicker } from "./EmojiKitchenPicker";
import {
  CustomEmojiMatches,
  CustomEmojiPicker,
  EmojiGrid,
  EmojiPickerRoot,
  EmojiSearchBar,
  useCustomEmojiSearch,
} from "./EmojiPicker";
import { RecentReactions } from "./RecentReactions";
import { useRecordEmoji } from "../../../lib/emoji/history";

type ReactionTab = "emoji" | "kitchen" | "custom";

const tabClassName =
  "flex-1 rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[active]:bg-background data-[active]:text-foreground data-[active]:shadow-sm";

export function ReactionPicker({
  open,
  onOpenChange,
  onSelect,
  purpose = "reaction",
}: {
  purpose?: "reaction" | "message";
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelect: (emoji: string) => void;
}) {
  const addRecentEmoji = useRecordEmoji();
  const { t } = useTranslation();
  const [tab, setTab] = useState<ReactionTab>("emoji");
  const [query, setQuery] = useState("");
  const changeOpen = (nextOpen: boolean) => {
    if (!nextOpen && purpose === "reaction") {
      setTab("emoji");
      setQuery("");
    }
    onOpenChange(nextOpen);
  };
  const select = (emoji: string) => {
    addRecentEmoji(emoji);
    onSelect(purpose === "message" ? kitchenShortcode(emoji) : emoji);
    changeOpen(false);
  };

  const [dialogElement, setDialogElement] = useState<HTMLDivElement | null>(null);
  const customEmojis = useCustomEmojiSearch(query);
  // While searching, emoji and custom emoji results are shown together.
  const searching = query.trim() !== "";
  const changeQuery = (value: string) => {
    setQuery(value);
    // Emoji Kitchen has no search; typing returns to the emoji results.
    if (value.trim() !== "" && tab === "kitchen") setTab("emoji");
  };
  const searchResults = (
    <EmojiGrid hideEmpty={searching && (customEmojis.filtered.length > 0 || customEmojis.failed)}>
      {searching ? <CustomEmojiMatches search={customEmojis} onSelect={select} /> : null}
    </EmojiGrid>
  );

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent
        ref={setDialogElement}
        className="flex h-[min(36rem,calc(100dvh-2rem))] flex-col gap-3 overflow-hidden sm:max-w-md"
      >
        <DialogHeader className="shrink-0 pr-8">
          <DialogTitle>
            {t(purpose === "message" ? "reactions.tabEmoji" : "reactions.add")}
          </DialogTitle>
          {tab === "kitchen" ? (
            <DialogDescription>{t("reactions.kitchenDescription")}</DialogDescription>
          ) : null}
        </DialogHeader>
        <EmojiPickerRoot
          onSelect={select}
          className="flex-1 gap-3"
          gridVisible={tab === "emoji" || (tab === "custom" && searching)}
        >
          <EmojiSearchBar query={query} onQueryChange={changeQuery} />
          <Tabs.Root
            value={tab}
            onValueChange={(value) => {
              if (value === "emoji" || value === "kitchen" || value === "custom") setTab(value);
            }}
            className="flex min-h-0 flex-1 flex-col gap-3"
          >
            <Tabs.List
              aria-label={t("reactions.picker")}
              className="flex shrink-0 gap-1 rounded-lg bg-muted p-1"
            >
              <Tabs.Tab value="emoji" className={tabClassName}>
                {t("reactions.tabEmoji")}
              </Tabs.Tab>
              <Tabs.Tab value="custom" className={tabClassName}>
                {t("home.customEmojis")}
              </Tabs.Tab>
              <Tabs.Tab value="kitchen" className={tabClassName}>
                {t("reactions.tabKitchen")}
              </Tabs.Tab>
            </Tabs.List>
            <Tabs.Panel value="emoji" className="flex min-h-0 flex-1 flex-col">
              {open && tab === "emoji" && !searching ? <RecentReactions onSelect={select} /> : null}
              {/* A panel that is leaving must not render a second copy of the results. */}
              {tab === "emoji" ? searchResults : null}
            </Tabs.Panel>
            <Tabs.Panel value="custom" className="min-h-0 flex-1">
              {tab !== "custom" ? null : searching ? (
                <div className="flex h-full min-h-0 flex-col">{searchResults}</div>
              ) : (
                <CustomEmojiPicker onSelect={select} search={customEmojis} />
              )}
            </Tabs.Panel>
            <Tabs.Panel value="kitchen" className="min-h-0 flex-1">
              <EmojiKitchenPicker
                active={open && tab === "kitchen"}
                onSelect={select}
                dialogElement={dialogElement}
              />
            </Tabs.Panel>
          </Tabs.Root>
        </EmojiPickerRoot>
      </DialogContent>
    </Dialog>
  );
}
