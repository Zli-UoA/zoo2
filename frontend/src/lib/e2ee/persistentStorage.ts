/**
 * 暗号ストア（OPFS と IndexedDB）を、ブラウザーが容量不足などで消さないよう頼む
 * （`navigator.storage.persist()`。docs/e2ee.md の「暗号ストア」）。
 *
 * 消されると端末の鍵を失い、新しい端末として登録し直すことになる。許されるかはブラウザーが決める
 * （Chrome はインストールやブックマークなど、Safari はホーム画面のアプリなど。Firefox は利用者に
 * 尋ねる）。結果で動きは変えない。ページを読み込んでいるあいだに 1 回だけ頼む（Worker では
 * `persist()` を使えないので、暗号ストアを開くタブの画面のスレッドで呼ぶ）。
 */
let requested: Promise<void> | null = null;

export function requestPersistentStorage(
  storage: Pick<StorageManager, "persist" | "persisted"> | undefined = globalThis.navigator
    ?.storage,
): Promise<void> {
  requested ??= (async () => {
    if (!storage?.persist || !storage.persisted) return;
    try {
      if (await storage.persisted()) return;
      await storage.persist();
    } catch {
      // 頼めなくても、暗号ストアはそのまま使う。
    }
  })();
  return requested;
}

/** テスト用：もう一度頼めるようにする。 */
export function resetPersistentStorageRequest(): void {
  requested = null;
}
