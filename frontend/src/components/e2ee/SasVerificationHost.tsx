import { useMemo, useSyncExternalStore } from "react";

import { useUsers } from "../../hooks/useRealtime";
import { useCollections } from "../../lib/electric/collections-context";
import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { startReauth } from "../../lib/e2ee/reauth";
import type { SasController, SasEntry } from "../../lib/e2ee/sasController";
import { SasVerificationView } from "./SasVerificationView";

const NO_ENTRIES: SasEntry[] = [];
const noop = () => () => {};

/**
 * 絵文字 SAS の検証（#368）のダイアログを、シェルのどの画面の上にも出す。届いた要求も、
 * 設定の「端末」タブから始めた検証も、ここに出る。一度に 1 つ（始まった順）だけを出す。
 */
export function SasVerificationHost() {
  const { sas } = useE2ee();
  const entries = useSyncExternalStore(
    sas?.subscribe ?? noop,
    sas?.getSnapshot ?? (() => NO_ENTRIES),
    () => NO_ENTRIES,
  );
  const entry = entries[0] ?? null;
  return sas ? <SasDialog sas={sas} entry={entry} /> : null;
}

function SasDialog({ sas, entry }: { sas: SasController; entry: SasEntry | null }) {
  const { config } = useCollections();
  const usersQuery = useUsers();
  const theirUserId = entry?.view.their_user_id ?? null;
  const name = useMemo(() => {
    if (!theirUserId) return "";
    const user = (usersQuery.data ?? []).find((row) => row.id === theirUserId);
    return user ? user.display_name || user.user_name : theirUserId;
  }, [theirUserId, usersQuery.data]);
  const txnId = entry?.view.transaction_id ?? null;
  return (
    <SasVerificationView
      entry={entry}
      name={name}
      handlers={{
        onAccept: () => {
          if (txnId) void sas.accept(txnId);
        },
        onCancel: () => {
          if (txnId) void sas.cancel(txnId);
        },
        // 再認証の別のウィンドウは、クリックの中で同期的に開く。相手の確認を待つあいだに
        // ログインし直してもらい、検証が終わったら、その ID トークンで保存する。
        onMatch: () => {
          if (!txnId) return;
          const reauth = startReauth(config);
          void sas.confirm(txnId, reauth());
        },
        onMismatch: () => {
          if (txnId) void sas.mismatch(txnId);
        },
        onRetrySave: () => {
          if (!txnId) return;
          const reauth = startReauth(config);
          void sas.retrySave(txnId, reauth());
        },
        onClose: () => {
          if (txnId) void sas.dismiss(txnId);
        },
      }}
    />
  );
}
