import { createContext, useContext } from "react";

export type OfflinePersistenceStatus = "loading" | "ready" | "unavailable" | "error";

export interface OfflineContextValue {
  /** ブラウザがオンラインか。 */
  isOnline: boolean;
  /** OPFS 永続化の状態。unavailable は非対応環境（メモリのみ）。 */
  persistenceStatus: OfflinePersistenceStatus;
  /** 初期化に失敗した理由（表示用）。 */
  persistenceError: string | null;
  /** ユーザー単位の名前空間。 */
  namespace: string;
  /** OPFS の初期化をやり直す。 */
  retryPersistence: () => void;
  /** ログアウト時にそのユーザーの OPFS データを削除する。 */
  clearUserData: () => Promise<void>;
}

const defaultValue: OfflineContextValue = {
  isOnline: true,
  persistenceStatus: "unavailable",
  persistenceError: null,
  namespace: "default",
  retryPersistence: () => {},
  clearUserData: async () => {},
};

export const OfflineContext = createContext<OfflineContextValue>(defaultValue);

/**
 * オフライン状態を読む。Provider の外（Storybook / 単体テスト）では
 * オンライン・永続化なしとして扱う。
 */
export function useOfflineStatus(): OfflineContextValue {
  return useContext(OfflineContext);
}
