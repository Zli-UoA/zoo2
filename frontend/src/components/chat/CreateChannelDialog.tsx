import { useTranslation } from "react-i18next";
import { useState, type FormEvent } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { getCollections } from "../../lib/electric/collections";
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

function describeError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/** チャンネル作成フォーム。`channels` コレクションへの insert で writers を通す。 */
export function CreateChannelDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { channels } = getCollections();
  const { t } = useTranslation();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const [name, setName] = useState("");
  const [isPrivate, setIsPrivate] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function handleOpenChange(next: boolean) {
    if (!next) {
      setName("");
      setIsPrivate(false);
      setError(null);
      setSubmitting(false);
    }
    onOpenChange(next);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    const channel_name = name.trim();
    if (!channel_name || submitting) {
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const transaction = channels.insert({
        id: crypto.randomUUID(),
        channel_name,
        description: null,
        created_user_id: currentUserId ?? "",
        is_private: isPrivate,
        created_at: new Date().toISOString(),
        updated_at: null,
        archive_at: null,
        deleted_at: null,
      });
      await transaction.when("settled");
      handleOpenChange(false);
    } catch (cause) {
      setError(describeError(cause));
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
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
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={isPrivate}
              onChange={(event) => setIsPrivate(event.target.checked)}
            />
            {t("channels.makePrivate")}
          </label>
          {error ? (
            <p className="text-xs text-destructive">{t("common.writeErrorDetail", { error })}</p>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={submitting || currentUserLoading || name.trim().length === 0}
            >
              {submitting ? t("channels.creating") : t("channels.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
