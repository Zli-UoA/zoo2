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
  /** Megolm のセッション鍵を受け取るたびに増える（復号できなかったメッセージを試し直す。#361）。 */
  roomKeysVersion: number;
  /**
   * ログアウトの前に、鍵バックアップにまだ送っていない受信用セッションの数を返す。
   * 暗号ストアがなければ 0。数える前に to-device の処理を止める（取りやめたら `cancelLogout`）。
   * 数えられなければ投げる。
   */
  pendingBackupCount(): Promise<number>;
  /** ログアウトを取りやめた。件数の確認のために奪ったロックを返す。 */
  cancelLogout(): void;
  /**
   * 鍵バックアップの送信待ちのセッションを送る（#360）。新しい受信用セッションを保存したあと
   * （送信用セッションの写しを含む）に呼ぶ。解錠していない端末では何もしない。
   */
  requestKeyBackup(): void;
  /** ログアウト：暗号ストアを消し、端末を削除する（失敗したら次のログインで消し直す）。 */
  clearForLogout(): Promise<void>;
}

export const E2eeContext = createContext<E2eeContextValue>({
  status: "idle",
  deviceId: null,
  worker: null,
  userId: null,
  roomKeysVersion: 0,
  pendingBackupCount: async () => 0,
  cancelLogout: () => {},
  requestKeyBackup: () => {},
  clearForLogout: async () => {},
});

export function useE2ee(): E2eeContextValue {
  return useContext(E2eeContext);
}
