import { useEffect } from "react";

import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { NOT_READY_STARTING } from "../../lib/e2ee/notReady";
import { DeviceLimit, DevicesSettings } from "./DevicesSettings";
import { EncryptionSettings } from "./EncryptionSettings";
import { E2EE_DEVICES_SECTION_ID, EncryptionTabView } from "./EncryptionTabView";

/**
 * 設定の「暗号化」タブ（暗号化と端末。#360、#362）。暗号ストアを開くまでは、開けていない理由を
 * 1 回だけ出し、各セクションは出さない（開いたら、各セクションが状態を読み込む）。
 *
 * `focusDevices` は、端末のセクションへ移るとき（古い `?tab=devices` のリンク）。移ったら
 * `onDevicesFocused` を呼ぶ。
 */
export function EncryptionTab({
  focusDevices = false,
  onDevicesFocused,
}: {
  focusDevices?: boolean;
  onDevicesFocused?: () => void;
}) {
  const { worker, userId, notReadyReason, deviceLimit } = useE2ee();
  const open = worker !== null && userId !== null;
  const notReady = open ? null : (notReadyReason ?? NOT_READY_STARTING);

  useEffect(() => {
    if (!focusDevices || !open) return;
    const frame = requestAnimationFrame(() => {
      document.getElementById(E2EE_DEVICES_SECTION_ID)?.scrollIntoView({ block: "start" });
      onDevicesFocused?.();
    });
    return () => cancelAnimationFrame(frame);
  }, [focusDevices, open, onDevicesFocused]);

  return (
    <EncryptionTabView
      notReady={notReady}
      deviceLimit={
        deviceLimit ? <DeviceLimit staleIds={deviceLimit.map((d) => d.device_id)} /> : null
      }
      encryption={<EncryptionSettings />}
      devices={<DevicesSettings />}
    />
  );
}
