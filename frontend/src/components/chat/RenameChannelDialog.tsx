import { useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { useCollections } from "../../lib/electric/collections-context";
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
import { Textarea } from "../ui/textarea";
import { channelDescriptionLength, MAX_CHANNEL_DESCRIPTION_LENGTH } from "./channelDescription";

/** 開くたびにマウントし、現在の名前と説明から編集を始める。 */
export function RenameChannelDialog({
  channel,
  onOpenChange,
}: {
  channel: ChannelRow;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const { channels } = useCollections();
  // 開いた時点の値を保持し、外部更新で prop が変わっても未編集フィールドは送らない。
  const initialName = useRef(channel.channel_name);
  const initialDescription = useRef(channel.description ?? "");
  const [name, setName] = useState(initialName.current);
  const [description, setDescription] = useState(initialDescription.current);
  const [submitting, setSubmitting] = useState(false);
  const pending = useRef(false);
  const [errorCause, setErrorCause] = useState<{ cause: unknown } | null>(null);
  const nextName = name.trim();
  const nextDescription = description.trim();
  const descriptionLength = channelDescriptionLength(nextDescription);
  const descriptionTooLong = descriptionLength > MAX_CHANNEL_DESCRIPTION_LENGTH;
  const nameChanged = nextName !== initialName.current.trim();
  const descriptionChanged = nextDescription !== initialDescription.current.trim();
  const hasChanges = nameChanged || descriptionChanged;

  function handleOpenChange(open: boolean) {
    if (!pending.current) {
      onOpenChange(open);
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!nextName || descriptionTooLong || !hasChanges || pending.current) {
      return;
    }
    pending.current = true;
    setSubmitting(true);
    setErrorCause(null);
    try {
      const transaction = channels.update(channel.id, (draft) => {
        if (nameChanged) {
          draft.channel_name = nextName;
        }
        if (descriptionChanged) {
          draft.description = nextDescription;
        }
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
      <DialogContent className="max-h-[85dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("channels.editTitle")}</DialogTitle>
          <DialogDescription id="rename-channel-help">{t("channels.editHelp")}</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-3" onSubmit={submit}>
          <input
            className={inputClassName()}
            aria-label={t("channels.newName")}
            aria-describedby="rename-channel-help"
            placeholder={t("channels.newName")}
            value={name}
            onChange={(event) => setName(event.target.value)}
            required
            disabled={submitting}
            autoFocus
          />
          <div className="flex flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-2">
              <label htmlFor="rename-channel-description" className="text-sm font-medium">
                {t("channels.descriptionLabel")}
              </label>
              <span
                className={
                  descriptionTooLong
                    ? "text-xs tabular-nums text-destructive"
                    : "text-xs tabular-nums text-muted-foreground"
                }
              >
                {t("channels.descriptionCounter", {
                  length: descriptionLength,
                  max: MAX_CHANNEL_DESCRIPTION_LENGTH,
                })}
              </span>
            </div>
            <Textarea
              id="rename-channel-description"
              className="max-h-40 overflow-y-auto"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder={t("channels.descriptionPlaceholder")}
              aria-invalid={descriptionTooLong}
              aria-describedby="rename-channel-description-help"
              disabled={submitting}
            />
            <p
              id="rename-channel-description-help"
              className={
                descriptionTooLong ? "text-xs text-destructive" : "text-xs text-muted-foreground"
              }
            >
              {descriptionTooLong
                ? t("channels.descriptionTooLong", {
                    length: descriptionLength,
                    max: MAX_CHANNEL_DESCRIPTION_LENGTH,
                  })
                : t("channels.descriptionHelp", { max: MAX_CHANNEL_DESCRIPTION_LENGTH })}
            </p>
          </div>
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
              disabled={submitting || !nextName || descriptionTooLong || !hasChanges}
            >
              {submitting ? t("channels.savingName") : t("channels.saveName")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
