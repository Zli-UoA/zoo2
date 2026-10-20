import { useTranslation } from "react-i18next";
import type { ChannelRow } from "../../lib/electric/types";
import { Dialog, DialogContent, DialogTitle } from "../ui/dialog";
import { MembersPanel } from "./MembersPanel";

/** 狭幅向けのメンバーダイアログ。中身は右ペインと同じ `MembersPanel`。 */
export function MembersDialog({
  channel,
  open,
  onOpenChange,
}: {
  channel: ChannelRow;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85dvh] flex-col gap-0 overflow-hidden p-0 sm:max-w-md">
        <DialogTitle className="sr-only">{t("members.title")}</DialogTitle>
        <MembersPanel channel={channel} className="flex" />
      </DialogContent>
    </Dialog>
  );
}
