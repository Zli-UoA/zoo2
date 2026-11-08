/**
 * 通話で使うマイク/スピーカーの選択状態と、デバイス一覧の取得。
 */
import { useEffect, useState } from "react";

export type CallDeviceKind = "mic" | "speaker";

const STORAGE_KEYS: Record<CallDeviceKind, string> = {
  mic: "zoo-call-mic",
  speaker: "zoo-call-speaker",
};
const CHANGE_EVENT = "zoo-call-device-change";

export interface CallDeviceOption {
  deviceId: string;
  label: string;
}

// 保存に失敗したときだけ使う、現在のページ内の選択値
const memoryOverride: Partial<Record<CallDeviceKind, string>> = {};

export function loadSelectedDevice(kind: CallDeviceKind): string {
  const override = memoryOverride[kind];
  if (override !== undefined) return override;
  try {
    return window.localStorage.getItem(STORAGE_KEYS[kind]) ?? "";
  } catch {
    return "";
  }
}

/** 空文字は「システム既定」。 */
export function saveSelectedDevice(kind: CallDeviceKind, deviceId: string): void {
  try {
    if (deviceId) window.localStorage.setItem(STORAGE_KEYS[kind], deviceId);
    else window.localStorage.removeItem(STORAGE_KEYS[kind]);
    delete memoryOverride[kind];
  } catch {
    // 保存できない環境でも、少なくとも現在のページでは選択を反映する
    memoryOverride[kind] = deviceId;
  }
  try {
    window.dispatchEvent(new Event(CHANGE_EVENT));
  } catch {}
}

export function subscribeSelectedDevices(onChange: () => void): () => void {
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || Object.values(STORAGE_KEYS).includes(e.key)) {
      // 他タブの保存が成功した以上、ページ内の暫定値は破棄する
      for (const kind of Object.keys(STORAGE_KEYS) as CallDeviceKind[]) {
        if (e.key === null || e.key === STORAGE_KEYS[kind]) delete memoryOverride[kind];
      }
      onChange();
    }
  };
  window.addEventListener(CHANGE_EVENT, onChange);
  window.addEventListener("storage", onStorage);
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange);
    window.removeEventListener("storage", onStorage);
  };
}

/** スピーカー選択 (setSinkId) に対応しているか。Safari などは非対応。 */
export function supportsSinkSelection(): boolean {
  return (
    typeof HTMLMediaElement !== "undefined" &&
    typeof (HTMLMediaElement.prototype as { setSinkId?: unknown }).setSinkId === "function"
  );
}

export async function applySink(audio: HTMLMediaElement, deviceId: string): Promise<void> {
  const el = audio as HTMLMediaElement & { setSinkId?: (id: string) => Promise<void> };
  if (typeof el.setSinkId !== "function") return;
  try {
    await el.setSinkId(deviceId);
  } catch {
    // デバイスが消えた場合などは既定の出力のままにする
  }
}

/** 一覧を整形する。ラベルは権限付与前は空なので連番の汎用名にする。 */
export function toDeviceOptions(
  devices: Pick<MediaDeviceInfo, "kind" | "deviceId" | "label">[],
  kind: "audioinput" | "audiooutput",
  fallbackLabel: (index: number) => string,
): CallDeviceOption[] {
  const out: CallDeviceOption[] = [];
  for (const d of devices) {
    // 権限がない状態では deviceId が空の項目が返る。選んでも既定と同じになるため除外する
    if (d.kind !== kind || !d.deviceId) continue;
    if (d.deviceId === "default" || d.deviceId === "communications") continue;
    out.push({ deviceId: d.deviceId, label: d.label || fallbackLabel(out.length + 1) });
  }
  return out;
}

export async function listAudioDevices(): Promise<MediaDeviceInfo[]> {
  try {
    if (!navigator.mediaDevices?.enumerateDevices) return [];
    return await navigator.mediaDevices.enumerateDevices();
  } catch {
    return [];
  }
}

/** デバイス一覧を購読する (devicechange で更新)。 */
export function useAudioDevices(): MediaDeviceInfo[] {
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  useEffect(() => {
    let active = true;
    const refresh = () => {
      void listAudioDevices().then((list) => {
        if (active) setDevices(list);
      });
    };
    refresh();
    const md = typeof navigator !== "undefined" ? navigator.mediaDevices : undefined;
    md?.addEventListener?.("devicechange", refresh);
    return () => {
      active = false;
      md?.removeEventListener?.("devicechange", refresh);
    };
  }, []);
  return devices;
}
