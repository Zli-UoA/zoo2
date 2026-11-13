import { createContext, useContext } from "react";

import type { E2eeDeviceStatus } from "./deviceManager";

export interface E2eeContextValue {
  /** `unsupported` は暗号ストアを置けないブラウザー（OPFS、Worker、Web Locks がない）。 */
  status: E2eeDeviceStatus | "unsupported" | "idle";
  deviceId: string | null;
  /**
   * ログアウトの前に、鍵バックアップにまだ送っていない受信用セッションの数を返す。
   * 暗号ストアがなければ 0。数える前に to-device の処理を止める（取りやめたら `cancelLogout`）。
   * 数えられなければ投げる。
   */
  pendingBackupCount(): Promise<number>;
  /** ログアウトを取りやめた。件数の確認のために奪ったロックを返す。 */
  cancelLogout(): void;
  /** ログアウト：暗号ストアを消し、端末を削除する（失敗したら次のログインで消し直す）。 */
  clearForLogout(): Promise<void>;
}

export const E2eeContext = createContext<E2eeContextValue>({
  status: "idle",
  deviceId: null,
  pendingBackupCount: async () => 0,
  cancelLogout: () => {},
  clearForLogout: async () => {},
});

export function useE2ee(): E2eeContextValue {
  return useContext(E2eeContext);
}
