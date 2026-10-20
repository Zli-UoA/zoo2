import { isChangeMessage, type ElectricCollectionUtils } from "@tanstack/electric-db-collection";
import type { Row } from "@tanstack/react-db";

import type { TxidMode } from "../config";

/** Electric の change message が持つ operation のうち、楽観更新と突き合わせる対象。 */
export type TxidOperation = "insert" | "update" | "delete";

type MatchFunction<T extends Row<unknown>> = Parameters<
  ElectricCollectionUtils<T>["awaitMatch"]
>[0];

/** Electric の change message を受け取り、目的の変更かどうかを返す述語。 */
export type ChangeMatcher<T extends Row<unknown>> = MatchFunction<T>;

/**
 * operation と行の判定を組み合わせた `awaitMatch` 用の述語を作る。
 *
 * `awaitTxId` が使えない環境（`txid` を返さないバックエンド）では、この述語で
 * Shape に流れてきた change message を照合する。論理削除は Electric 上は
 * `delete` ではなく `update`（`deleted_at` の更新）として届く点に注意する。
 */
export function changeMessageMatcher<T extends Row<unknown>>(
  operation: TxidOperation,
  matches: (value: T) => boolean,
): ChangeMatcher<T> {
  return (message) =>
    isChangeMessage(message) && message.headers.operation === operation && matches(message.value);
}

export interface TxidSyncTarget<T extends Row<unknown>> {
  awaitTxId: ElectricCollectionUtils<T>["awaitTxId"];
  awaitMatch: ElectricCollectionUtils<T>["awaitMatch"];
}

export interface SyncTxidOptions<T extends Row<unknown>> {
  mode: TxidMode;
  /** REST エンベロープの `txid`。Electric は number を要求するため数値化する。 */
  txid: string | null;
  target: TxidSyncTarget<T>;
  /** `awaitMatch` モードで使う照合述語。 */
  match: ChangeMatcher<T>;
  timeout?: number;
}

/**
 * 書き込みレスポンスの `txid` と Electric ストリームを突き合わせる。
 *
 * - `awaitTxId`: `txid` がそのコレクションの Shape に流れてくるまで待つ。待機には
 *   対象コレクションが live query で購読されている必要がある。
 * - `awaitMatch`: `txid` 非対応環境向け。operation と行の一致で待つ。
 * - `skip`: 待機せず即座に解決する。
 *
 * タイムアウトや失敗はそのまま伝播させる。呼び出し元（TanStack DB の
 * mutation ハンドラ）では、ここで throw すると楽観状態がロールバックされる。
 */
export async function syncTxid<T extends Row<unknown>>(options: SyncTxidOptions<T>): Promise<void> {
  const { mode, txid, target, match, timeout } = options;
  // Idempotent no-ops return null and emit no change or txid to wait for.
  if (mode === "skip" || txid === null) {
    return;
  }
  if (mode === "awaitTxId") {
    await target.awaitTxId(Number(txid), timeout);
    return;
  }
  await target.awaitMatch(match, timeout);
}
