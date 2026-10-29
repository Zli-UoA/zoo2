import { useTranslation } from "react-i18next";
import { useState, type FormEvent } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useCollections } from "../../lib/electric/collections-context";
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

/** チャンネル作成フォーム。`channels` コレクションへの insert で writers を通す。 */
export function CreateChannelDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { channels } = useCollections();
  const { t } = useTranslation();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [isPrivate, setIsPrivate] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  // 言語を切り替えても文言が追従するよう、翻訳前の原因を保持する。
  const [errorCause, setErrorCause] = useState<{ cause: unknown } | null>(null);
  const trimmedDescription = description.trim();
  const descriptionLength = channelDescriptionLength(trimmedDescription);
  const descriptionTooLong = descriptionLength > MAX_CHANNEL_DESCRIPTION_LENGTH;

  function handleOpenChange(next: boolean) {
    if (!next) {
      setName("");
      setDescription("");
      setIsPrivate(false);
      setErrorCause(null);
      setSubmitting(false);
    }
    onOpenChange(next);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    const channel_name = name.trim();
    if (!channel_name || descriptionTooLong || submitting) {
      return;
    }
    setSubmitting(true);
    setErrorCause(null);
    try {
      const transaction = channels.insert({
        id: crypto.randomUUID(),
        channel_name,
        description: trimmedDescription || null,
        created_user_id: currentUserId ?? "",
        is_private: isPrivate,
        is_dm: false,
        created_at: new Date().toISOString(),
        updated_at: null,
        archive_at: null,
        deleted_at: null,
      });
      await transaction.when("settled");
      handleOpenChange(false);
    } catch (cause) {
      setErrorCause({ cause });
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-h-[85dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("channels.createTitle")}</DialogTitle>
          <DialogDescription>{t("channels.visibilityHelp")}</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-3" onSubmit={submit}>
          <input
            className={inputClassName()}
            placeholder={t("channels.newName")}
            value={name}
            onChange={(event) => setName(event.target.value)}
            aria-label={t("channels.newName")}
            autoFocus
          />
          <div className="flex flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-2">
              <label htmlFor="create-channel-description" className="text-sm font-medium">
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
              id="create-channel-description"
              className="max-h-40 overflow-y-auto"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder={t("channels.descriptionPlaceholder")}
              aria-invalid={descriptionTooLong}
              aria-describedby="create-channel-description-help"
              disabled={submitting}
            />
            <p
              id="create-channel-description-help"
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
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={isPrivate}
              onChange={(event) => setIsPrivate(event.target.checked)}
            />
            {t("channels.makePrivate")}
          </label>
          {errorCause ? (
            <p role="alert" className="text-xs text-destructive">
              {t("common.writeErrorDetail", {
                error: localizeApiError(errorCause.cause, t),
              })}
            </p>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={
                submitting || currentUserLoading || name.trim().length === 0 || descriptionTooLong
              }
            >
              {submitting ? t("channels.creating") : t("channels.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
