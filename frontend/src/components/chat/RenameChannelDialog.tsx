import { useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { getCollections } from "../../lib/electric/collections";
import type { ChannelRow } from "../../lib/electric/types";
import { localizeApiError } from "../../lib/i18n/apiError";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";

/** 開くたびにマウントし、現在の名前から編集を始める。 */
export function RenameChannelDialog({
  channel,
  onOpenChange,
}: {
  channel: ChannelRow;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(channel.channel_name);
  const [submitting, setSubmitting] = useState(false);
  const pending = useRef(false);
  const [errorCause, setErrorCause] = useState<{ cause: unknown } | null>(null);
  const nextName = name.trim();

  function handleOpenChange(open: boolean) {
    if (!pending.current) {
      onOpenChange(open);
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!nextName || nextName === channel.channel_name || pending.current) {
      return;
    }
    pending.current = true;
    setSubmitting(true);
    setErrorCause(null);
    try {
      const { channels } = getCollections();
      const transaction = channels.update(channel.id, (draft) => {
        draft.channel_name = nextName;
      });
      await transaction.when("settled");
      onOpenChange(false);
    } catch (cause) {
      setErrorCause({ cause });
    } finally {
      pending.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog open onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("channels.renameTitle")}</DialogTitle>
          <DialogDescription id="rename-channel-help">{t("channels.renameHelp")}</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-3" onSubmit={submit}>
          <input
            className={inputClassName()}
            aria-label={t("channels.newName")}
            aria-describedby="rename-channel-help"
            placeholder={t("channels.newName")}
            value={name}
            onChange={(event) => setName(event.target.value)}
            maxLength={80}
            pattern={"[a-z0-9_\\-]+"}
            required
            disabled={submitting}
            autoFocus
          />
          {errorCause ? (
            <p role="alert" className="text-xs text-destructive">
              {t("common.writeErrorDetail", { error: localizeApiError(errorCause.cause, t) })}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={submitting}
              onClick={() => handleOpenChange(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={submitting || !nextName || nextName === channel.channel_name}
            >
              {submitting ? t("channels.savingName") : t("channels.saveName")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
