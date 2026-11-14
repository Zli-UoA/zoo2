import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { createConfiguredApiClient } from "../../lib/api/client";
import { encryptChannel } from "../../lib/api/encryptChannel";
import { useCollections } from "../../lib/electric/collections-context";
import { changeMessageMatcher, syncTxid } from "../../lib/electric/txid";
import type { ChannelRow } from "../../lib/electric/types";
import { localizeApiError } from "../../lib/i18n/apiError";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "../ui/alert-dialog";

/** Shape への反映を待つ上限。届かなくても切り替え自体は成功しているので、失敗にはしない。 */
const SHAPE_SYNC_TIMEOUT_MS = 5000;

/**
 * 既存の非公開チャンネルか DM を暗号化に切り替える確認の画面（#367。docs/e2ee.md の「既存のチャンネルの暗号化」）。
 *
 * 元に戻せないこと、これまでのメッセージは平文のまま残ること、Home や ActivityPub に出した投稿は取り消せない
 * ことを示してから切り替える。失敗は画面の中に出す（App や Webhook を外すなど、利用者が直せるものがあるため）。
 */
export function EncryptChannelDialog({
  channel,
  open,
  onOpenChange,
}: {
  channel: ChannelRow;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const collections = useCollections();
  const client = useMemo(() => createConfiguredApiClient(collections.config), [collections]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isDm = channel.is_dm;

  function handleOpenChange(next: boolean) {
    if (pending) return;
    if (!next) setError(null);
    onOpenChange(next);
  }

  async function confirm() {
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      const response = await encryptChannel(client, channel.id);
      try {
        await syncTxid({
          mode: collections.config.txidMode,
          txid: response.txid,
          target: collections.channels.utils,
          match: changeMessageMatcher<ChannelRow>(
            "update",
            (row) => row.id === channel.id && row.encrypted === true,
          ),
          timeout: SHAPE_SYNC_TIMEOUT_MS,
        });
      } catch (cause) {
        console.warn("Shape 反映の待機に失敗しました", cause);
      }
      toast.success(t("e2ee.convert.success"));
      onOpenChange(false);
    } catch (cause) {
      setError(localizeApiError(cause, t));
    } finally {
      setPending(false);
    }
  }

  return (
    <EncryptChannelConfirm
      open={open}
      onOpenChange={handleOpenChange}
      isDm={isDm}
      pending={pending}
      error={error}
      onConfirm={() => void confirm()}
    />
  );
}

/**
 * 暗号化の確認の画面の表示だけ（Storybook とテストで使う）。`EncryptChannelDialog` が API を呼ぶ。
 */
export function EncryptChannelConfirm({
  open,
  onOpenChange,
  isDm,
  pending = false,
  error = null,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  isDm: boolean;
  pending?: boolean;
  error?: string | null;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const points = isDm
    ? (["future", "earlier", "irreversible"] as const)
    : (["future", "earlier", "published", "integrations", "irreversible"] as const);

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!pending) onOpenChange(next);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {isDm ? t("e2ee.convert.titleDm") : t("e2ee.convert.title")}
          </AlertDialogTitle>
          <AlertDialogDescription>{t("e2ee.convert.description")}</AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
          {points.map((point) => (
            <li key={point}>{t(`e2ee.convert.points.${point}`)}</li>
          ))}
        </ul>
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel className="min-h-11 md:min-h-8" disabled={pending}>
            {t("common.cancelShort")}
          </AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            className="min-h-11 md:min-h-8"
            disabled={pending}
            onClick={onConfirm}
          >
            {pending ? t("e2ee.convert.pending") : t("e2ee.convert.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
