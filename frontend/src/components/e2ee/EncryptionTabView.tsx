import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ShieldCheckIcon } from "lucide-react";

import type { E2eeNotReadyReason } from "../../lib/e2ee/notReady";
import { E2eeNotReadyNotice } from "./DevicesSettingsView";

/** 端末のセクションの id（`?tab=devices` の古いリンクから、ここへ移す）。 */
export const E2EE_DEVICES_SECTION_ID = "e2ee-devices";

/**
 * 設定の「暗号化」タブ。上から、状態と解錠、鍵（パスキー、リカバリーキー）、端末（一覧、削除、
 * 失効）、他のユーザーの検証の順に並べる。前の 2 つは `encryption`（`EncryptionSettingsView`）、
 * 後の 2 つは `devices`（`DevicesSettingsView`）が出す。
 *
 * 暗号ストアを開けていないあいだは、どのセクションも使えないので、その理由だけを 1 回出す。端末が
 * 20 台の上限で登録できないときは、削除する端末を選ぶ画面（`deviceLimit`）を理由のすぐ下に出す。
 */
export function EncryptionTabView({
  notReady,
  deviceLimit,
  encryption,
  devices,
}: {
  notReady: E2eeNotReadyReason | null;
  deviceLimit?: ReactNode;
  encryption: ReactNode;
  devices: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <div className="rounded-lg border p-4">
        <p className="flex items-center gap-2 text-sm font-semibold">
          <ShieldCheckIcon aria-hidden="true" className="size-5 text-muted-foreground" />
          {t("e2ee.title")}
        </p>
        <p className="mt-1 text-xs text-muted-foreground">{t("e2ee.description")}</p>
      </div>
      {notReady ? (
        <>
          <E2eeNotReadyNotice reason={notReady} />
          {deviceLimit ?? null}
        </>
      ) : (
        <>
          {encryption}
          <div id={E2EE_DEVICES_SECTION_ID} className="scroll-mt-4">
            {devices}
          </div>
        </>
      )}
    </div>
  );
}
