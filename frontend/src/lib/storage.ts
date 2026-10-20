/** Storage may be blocked at property access as well as at getItem/setItem. */
export function readStorageValue(
  key: string,
  storage?: Pick<Storage, "getItem"> | null,
): string | null {
  try {
    return (storage === undefined ? window.localStorage : storage)?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function writeStorageValue(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Keep the in-memory selection when persistence is unavailable.
  }
}

/** A null key represents localStorage.clear() and invalidates every preference. */
export function subscribeStorage(key: string, onChange: () => void): () => void {
  const listener = (event: StorageEvent) => {
    if (event.key === key || event.key === null) onChange();
  };
  window.addEventListener("storage", listener);
  return () => window.removeEventListener("storage", listener);
}
