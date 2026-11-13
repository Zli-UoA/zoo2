import { createContext, useContext } from "react";

import type { CryptoWorker } from "./cryptoWorker";
import type { E2eeDeviceStatus } from "./deviceManager";

export interface E2eeContextValue {
  /** `unsupported` は暗号ストアを置けないブラウザー（OPFS、Worker、Web Locks がない）。 */
  status: E2eeDeviceStatus | "unsupported" | "idle";
  deviceId: string | null;
  /**
   * 暗号ストアを開いた Worker（このタブが Web Locks を取り、端末の登録が済んだときだけ）。
   * 設定の「暗号化」タブ（初回設定と解錠。#360）が使う。
   */
  worker: CryptoWorker | null;
  userId: string | null;
  /**
   * ログアウトの前に、鍵バックアップにまだ送っていない受信用セッションの数を返す。
   * 暗号ストアがなければ 0。
   */
  pendingBackupCount(): Promise<number>;
  /** ログアウト：暗号ストアを消し、端末を削除する（失敗したら次のログインで消し直す）。 */
  clearForLogout(): Promise<void>;
}

export const E2eeContext = createContext<E2eeContextValue>({
  status: "idle",
  deviceId: null,
  worker: null,
  userId: null,
  pendingBackupCount: async () => 0,
  clearForLogout: async () => {},
});

export function useE2ee(): E2eeContextValue {
  return useContext(E2eeContext);
}
