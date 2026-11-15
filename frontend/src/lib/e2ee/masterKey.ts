import type { JsonObject } from "@zoo/crypto-wasm";

/**
 * マスター鍵の JSON（`keys/query` の `master_keys` の値）から、公開鍵（パディングなしの base64）を
 * 取り出す。`keys` に `ed25519:<公開鍵>` がちょうど 1 つあるときだけ返す（zoo-crypto の検証と同じ形）。
 * 再検証の確認の画面に出した鍵と、署名する鍵を比べるのに使う。
 */
export function masterPublicKey(masterKey: JsonObject | undefined): string | null {
  const keys = masterKey?.keys;
  if (!keys || typeof keys !== "object" || Array.isArray(keys)) return null;
  const entries = Object.entries(keys as Record<string, unknown>).filter(([id]) =>
    id.startsWith("ed25519:"),
  );
  if (entries.length !== 1) return null;
  const [id, value] = entries[0]!;
  return typeof value === "string" && id === `ed25519:${value}` ? value : null;
}

/**
 * 鍵の指紋の表示。公開鍵（base64）を 4 文字ずつに区切る（チャンネルの外で読み上げて比べやすいように）。
 */
export function formatKeyFingerprint(publicKey: string): string {
  return publicKey.match(/.{1,4}/g)?.join(" ") ?? publicKey;
}
