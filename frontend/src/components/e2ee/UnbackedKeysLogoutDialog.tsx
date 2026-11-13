import { useTranslation } from "react-i18next";

import { ConfirmActionDialog } from "../ui/confirm-action-dialog";

/**
 * 鍵バックアップに送れていない鍵があるときの、ログアウトの確認（docs/e2ee.md の
 * 「ログアウトと端末の削除」）。黙って消さず、利用者が取りやめられるようにする。
 */
export function UnbackedKeysLogoutDialog({
  count,
  pending = false,
  onCancel,
  onConfirm,
}: {
  /** 送れていない鍵の数。`null` なら出さない。負なら数えられなかった。 */
  count: number | null;
  pending?: boolean;
  onCancel: () => void;
  onConfirm: () => void | Promise<void>;
}) {
  const { t } = useTranslation();
  return (
    <ConfirmActionDialog
      open={count !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title={t("e2ee.logout.unbackedTitle")}
      description={
        count !== null && count < 0
          ? t("e2ee.logout.unknownDescription")
          : t("e2ee.logout.unbackedDescription", { count: count ?? 0 })
      }
      confirmLabel={t("e2ee.logout.confirm")}
      onConfirm={onConfirm}
      pending={pending}
    />
  );
}
