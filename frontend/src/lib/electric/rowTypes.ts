import type { EncryptedMessage } from "@zoo/crypto-wasm";
import type { persistedCollectionOptions } from "@tanstack/db-sqlite-persistence-core";

import type { MegolmCiphertext, TableName, TableRowMap } from "./types";

/**
 * 永続化ラッパーの入力型。`persistedCollectionOptions` の最後のオーバーロードに合わせる。
 *
 * `@tanstack/db-sqlite-persistence-core` 0.4.12 は `PersistedSyncWrappedOptions` を要求するが、
 * `@tanstack/electric-db-collection` 0.6.3 のオプションは `sync` の形が異なり直接渡せない。
 * ここでは参照をそのまま渡すだけなので、transformer / 索引 / 永続化の設定は変わらない。
 */
type PersistedOptions = Parameters<typeof persistedCollectionOptions>[0];

export function asPersistedOptions(options: object): PersistedOptions {
  return options as unknown as PersistedOptions;
}

/**
 * 永続化ラッパーの戻り値を、呼び出し側が期待する Electric コレクションのオプション型へ戻す。
 *
 * 0.4.12 の戻り値には `persistence` / `utils` が足され、`@tanstack/react-db` 0.5.12 の
 * `createCollection` に渡す型とは一致しない。値を触らず型だけを合わせる。
 */
export function asElectricOptions<TOptions extends object>(options: object): TOptions {
  return options as unknown as TOptions;
}

/**
 * Electric の生行 (`Record<string, unknown>`) をアプリの行型へ戻す。
 *
 * `@tanstack/electric-db-collection` 0.6.3 の transformer は列ごとの型を推論せず、
 * `@tanstack/react-db` 0.5.12 のコレクションも生行のまま扱うため、境界が必要。
 * 値の正規化は `coerceElectricRow` が行い、ここでは型だけを合わせる。
 */
export function asTableRow<K extends TableName>(row: Record<string, unknown>): TableRowMap[K] {
  return row as unknown as TableRowMap[K];
}

/**
 * メッセージ行の暗号文 (Electric 行型の `MegolmCiphertext`) を wasm 側の `JsonObject` へ渡す。
 *
 * `@zoo/crypto-wasm` が wasm-bindgen 0.2.129 で生成する JSON 型は構造が近いのに代入できない。
 * 暗号文がある行だけを呼び出し元で選別して渡す。値をそのまま渡すだけなので、復号対象の暗号文は変わらない。
 */
export function asWasmCiphertext(value: MegolmCiphertext): EncryptedMessage["ciphertext"] {
  return value as unknown as EncryptedMessage["ciphertext"];
}
