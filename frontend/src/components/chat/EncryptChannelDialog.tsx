import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { createConfiguredApiClient } from "../../lib/api/client";
import { encryptChannel } from "../../lib/api/encryptChannel";
import { useCollections } from "../../lib/electric/collections-context";
import { noteEncryptedChannel } from "../../lib/e2ee/encryptionGuard";
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

  // 確認を開いたときの対象に固定する。開いたまま対象が変わったら、確認を閉じる（移動先のチャンネルを、確かめずに
  // 戻せない形で切り替えない）。`ChannelScreen` は `ChannelHeader` をチャンネルごとに作り直すので、ふつうは確認ごと
  // なくなるが、この部品だけでも守る。
  const [openedFor, setOpenedFor] = useState<string | null>(null);
  if (open && openedFor === null) setOpenedFor(channel.id);
  if (!open && openedFor !== null) setOpenedFor(null);
  const channelId = openedFor ?? channel.id;
  const targetChanged = open && openedFor !== null && openedFor !== channel.id;
  useEffect(() => {
    if (targetChanged) {
      setError(null);
      onOpenChange(false);
    }
  }, [targetChanged, onOpenChange]);

  async function confirm() {
    if (pending || targetChanged) return;
    setPending(true);
    setError(null);
    try {
      const response = await encryptChannel(client, channelId);
      // 応答で暗号化を確かめたら、Shape の反映を待たずに覚える。表示中の `MessagePane` は描き直して送信を止め、
      // Shape が遅れても（反映の待ちが失敗しても）平文を送らない（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
      if (response.data.encrypted === true) noteEncryptedChannel(channelId);
      try {
        await syncTxid({
          mode: collections.config.txidMode,
          txid: response.txid,
          target: collections.channels.utils,
          match: changeMessageMatcher<ChannelRow>(
            "update",
            (row) => row.id === channelId && row.encrypted === true,
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
