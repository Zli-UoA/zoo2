import { useCallback, useEffect, useState } from "react";

import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { NOT_READY_STARTING } from "../../lib/e2ee/notReady";
import { DeviceLimit, DevicesSettings } from "./DevicesSettings";
import { EncryptionSettings } from "./EncryptionSettings";
import { E2EE_DEVICES_SECTION_ID, EncryptionTabView } from "./EncryptionTabView";

/**
 * 設定の「暗号化」タブ（暗号化と端末。#360、#362）。暗号ストアを開くまでは、開けていない理由を
 * 1 回だけ出し、各セクションは出さない（開いたら、各セクションが状態を読み込む）。
 *
 * 2 つのセクションは同じタブに並ぶので、片方の操作で変わったものを、もう片方に読み直してもらう。
 * - 解錠か初回設定を終えたら、端末と他のユーザーの検証を読み直す（解錠したかで、失効と再検証を出す）。
 * - 失効を終えたら、鍵スロット（リカバリーキー、パスキー）を読み直す。
 *
 * `focusDevices` は、端末のセクションへ移るとき（古い `?tab=devices` のリンク）。上のセクションの
 * 高さが変わらないよう、両方のセクションを読み込み終えてから移り、`onDevicesFocused` を呼ぶ。
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

  const [devicesRefresh, setDevicesRefresh] = useState(0);
  const [encryptionRefresh, setEncryptionRefresh] = useState(0);
  const [encryptionSettled, setEncryptionSettled] = useState(false);
  const [devicesSettled, setDevicesSettled] = useState(false);
  const onUnlocked = useCallback(() => setDevicesRefresh((value) => value + 1), []);
  const onKeysChanged = useCallback(() => setEncryptionRefresh((value) => value + 1), []);

  // 暗号ストアを閉じたら、各セクションは外れる。開き直したら、読み込みを待ち直す。
  useEffect(() => {
    if (open) return;
    setEncryptionSettled(false);
    setDevicesSettled(false);
  }, [open]);

  const settled = open && encryptionSettled && devicesSettled;
  useEffect(() => {
    if (!focusDevices || !settled) return;
    const frame = requestAnimationFrame(() => {
      document.getElementById(E2EE_DEVICES_SECTION_ID)?.scrollIntoView({ block: "start" });
      onDevicesFocused?.();
    });
    return () => cancelAnimationFrame(frame);
  }, [focusDevices, settled, onDevicesFocused]);

  return (
    <EncryptionTabView
      notReady={notReady}
      deviceLimit={
        deviceLimit ? <DeviceLimit staleIds={deviceLimit.map((d) => d.device_id)} /> : null
      }
      encryption={
        <EncryptionSettings
          refreshKey={encryptionRefresh}
          onUnlocked={onUnlocked}
          onSettled={setEncryptionSettled}
        />
      }
      devices={
        <DevicesSettings
          refreshKey={devicesRefresh}
          onKeysChanged={onKeysChanged}
          onSettled={setDevicesSettled}
        />
      }
    />
  );
}
