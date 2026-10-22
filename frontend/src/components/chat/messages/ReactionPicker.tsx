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
import { EmojiPicker } from "./EmojiPicker";

type ReactionTab = "emoji" | "kitchen";

const tabClassName =
  "flex-1 rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[active]:bg-background data-[active]:text-foreground data-[active]:shadow-sm";

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
  const [tab, setTab] = useState<ReactionTab>("emoji");
  const [dialogElement, setDialogElement] = useState<HTMLDivElement | null>(null);
  const select = (emoji: string) => {
    onSelect(emoji);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        ref={setDialogElement}
        className="flex h-[min(36rem,calc(100dvh-2rem))] flex-col gap-3 overflow-hidden sm:max-w-md"
      >
        <DialogHeader className="shrink-0 pr-8">
          <DialogTitle>{t("reactions.add")}</DialogTitle>
          <DialogDescription>
            {tab === "kitchen"
              ? t("reactions.kitchenDescription")
              : t("reactions.emojiDescription")}
          </DialogDescription>
        </DialogHeader>
        <Tabs.Root
          value={tab}
          onValueChange={(value) => {
            if (value === "emoji" || value === "kitchen") setTab(value);
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
            <Tabs.Tab value="kitchen" className={tabClassName}>
              {t("reactions.tabKitchen")}
            </Tabs.Tab>
          </Tabs.List>
          <Tabs.Panel value="emoji" className="min-h-0 flex-1">
            <EmojiPicker onSelect={select} />
          </Tabs.Panel>
          <Tabs.Panel value="kitchen" className="min-h-0 flex-1">
            <EmojiKitchenPicker
              active={open && tab === "kitchen"}
              onSelect={select}
              dialogElement={dialogElement}
            />
          </Tabs.Panel>
        </Tabs.Root>
      </DialogContent>
    </Dialog>
  );
}
