import { Input } from "../ui/input";
import { useTranslation } from "react-i18next";
import { LockIcon } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useCollections } from "../../lib/electric/collections-context";
import { localizeApiError } from "../../lib/i18n/apiError";
import { useMediaQuery } from "../layout/useMediaQuery";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";
import { Textarea } from "../ui/textarea";
import { channelDescriptionLength, MAX_CHANNEL_DESCRIPTION_LENGTH } from "./channelDescription";
import { channelNameLength, MAX_CHANNEL_NAME_LENGTH, validateChannelName } from "./channelName";

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
  const [isNsfw, setIsNsfw] = useState(false);
  const [encrypted, setEncrypted] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [focusErrorCount, setFocusErrorCount] = useState(0);
  const nameErrorRef = useRef<HTMLParagraphElement>(null);
  const finePointer = useMediaQuery("(pointer: fine)");
  // 言語を切り替えても文言が追従するよう、翻訳前の原因を保持する。
  const [errorCause, setErrorCause] = useState<{ cause: unknown } | null>(null);
  const trimmedName = name.trim();
  const nameError = validateChannelName(trimmedName);
  const showNameError = nameError !== null && (trimmedName.length > 0 || submitted);
  const trimmedDescription = description.trim();
  const descriptionLength = channelDescriptionLength(trimmedDescription);
  const descriptionTooLong = descriptionLength > MAX_CHANNEL_DESCRIPTION_LENGTH;

  // エラー文の描画を待ってからフォーカスし、スクリーンリーダーに読み上げさせる。
  useEffect(() => {
    if (focusErrorCount > 0) {
      nameErrorRef.current?.focus();
    }
  }, [focusErrorCount]);

  function nameErrorText(): string {
    switch (nameError) {
      case "empty":
        return t("channels.nameRequired");
      case "tooLong":
        return t("channels.nameTooLong", {
          length: channelNameLength(trimmedName),
          max: MAX_CHANNEL_NAME_LENGTH,
        });
      case "reserved":
        return t("channels.nameReserved", { name: trimmedName });
      case "invalidCharacters":
        return t("channels.invalidName");
      default:
        return "";
    }
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      setName("");
      setDescription("");
      setIsPrivate(false);
      setIsNsfw(false);
      setEncrypted(false);
      setErrorCause(null);
      setSubmitting(false);
      setSubmitted(false);
      setFocusErrorCount(0);
    }
    onOpenChange(next);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (nameError || descriptionTooLong || submitting) {
      setSubmitted(true);
      if (nameError) {
        setFocusErrorCount((count) => count + 1);
      }
      return;
    }
    setSubmitting(true);
    setErrorCause(null);
    try {
      const transaction = channels.insert({
        id: crypto.randomUUID(),
        channel_name: trimmedName,
        description: trimmedDescription || null,
        created_user_id: currentUserId ?? "",
        is_private: isPrivate,
        is_nsfw: isNsfw,
        is_dm: false,
        created_at: new Date().toISOString(),
        updated_at: null,
        archive_at: null,
        imported_from_slack: false,
        slack_import_kind: null,
        slack_imported_at: null,
        deleted_at: null,
        // 暗号化できるのは非公開チャンネルだけ（公開にしたら外す）。
        encrypted: isPrivate && encrypted,
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
        <form autoComplete="off" className="flex flex-col gap-3" onSubmit={submit}>
          <div className="flex flex-col gap-1.5">
            <Input
              name="channel_name"
              placeholder={t("channels.newName")}
              value={name}
              onChange={(event) => setName(event.target.value)}
              aria-label={t("channels.newName")}
              aria-invalid={showNameError}
              aria-required
              aria-describedby="create-channel-name-help"
              disabled={submitting}
              autoFocus={finePointer}
            />
            <p
              id="create-channel-name-help"
              ref={nameErrorRef}
              tabIndex={-1}
              className={
                showNameError ? "text-xs text-destructive" : "text-xs text-muted-foreground"
              }
            >
              {showNameError ? nameErrorText() : t("channels.editHelp")}
            </p>
          </div>
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
            <Input
              type="checkbox"
              checked={isPrivate}
              onChange={(event) => setIsPrivate(event.target.checked)}
            />
            {t("channels.makePrivate")}
          </label>
          <div className="flex flex-col gap-1">
            <label className="flex items-center gap-2 text-sm">
              <Input
                type="checkbox"
                checked={isPrivate && encrypted}
                disabled={submitting || !isPrivate}
                aria-describedby="create-channel-encrypted-help"
                onChange={(event) => setEncrypted(event.target.checked)}
              />
              <LockIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
              {t("e2ee.encryptChannel")}
            </label>
            <p id="create-channel-encrypted-help" className="text-xs text-muted-foreground">
              {isPrivate ? t("e2ee.encryptChannelHelp") : t("e2ee.encryptChannelPrivateOnly")}
            </p>
          </div>
          <label className="flex items-center gap-2 text-sm">
            <Input
              type="checkbox"
              checked={isNsfw}
              disabled={submitting}
              onChange={(event) => setIsNsfw(event.target.checked)}
            />
            {t("channels.nsfw")}
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
              // 不正な名前でも押せるようにして、送信時にエラー欄へ案内できるようにする。
              disabled={submitting || currentUserLoading || descriptionTooLong}
            >
              {submitting ? t("channels.creating") : t("channels.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
