/** 暗号ストアを置けるか（OPFS、Worker、Web Locks、IndexedDB、WebCrypto）。 */
export function supportsCryptoStore(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof Worker !== "undefined" &&
    typeof indexedDB !== "undefined" &&
    typeof navigator !== "undefined" &&
    typeof navigator.storage?.getDirectory === "function" &&
    typeof navigator.locks?.request === "function" &&
    typeof crypto?.subtle !== "undefined"
  );
}
