import { useTranslation } from "react-i18next";
import { getCollections } from "../lib/electric/collections";

function StatusItem({ label, value }: { label: string; value: string }) {
  return (
    <span className="max-w-full rounded bg-muted px-2 py-1 break-all">
      {label}: {value}
    </span>
  );
}

/** Shape の接続モードと書き込みの待ち方、現在の開発ユーザーを表示する。 */
export function SyncStatus() {
  const { t } = useTranslation();
  const { config } = getCollections();
  return (
    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
      <StatusItem label="Shape" value={config.shapeMode} />
      <StatusItem label="txid" value={config.txidMode} />
      <StatusItem
        label={t("sync.user")}
        value={config.oidc ? t("sync.oidc") : (config.devUserId ?? t("sync.unset"))}
      />
    </div>
  );
}
