import { createContext, useContext } from "react";

import type { CryptoWorker } from "./cryptoWorker";

/**
 * 暗号ストアを開いた端末（#359 の端末の登録が済んだもの）。設定の「暗号化」タブ（#360）が使う。
 *
 * Web Locks を取れたタブで暗号ストアを開く側（`E2eeDeviceManager` を動かす Provider）が値を入れる。
 * 値がない（まだ開いていない、ほかのタブが開いている）あいだは、タブに「準備ができていない」と出す。
 */
export interface E2eeDevice {
  worker: CryptoWorker;
  userId: string;
  deviceId: string;
}

export const E2eeDeviceContext = createContext<E2eeDevice | null>(null);

export function useE2eeDevice(): E2eeDevice | null {
  return useContext(E2eeDeviceContext);
}
