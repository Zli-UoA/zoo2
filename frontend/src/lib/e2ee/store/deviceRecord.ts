import { withStore } from "./storageKey";

/**
 * 暗号ストアが持つ（持っていた）端末の記録（docs/e2ee.md の「暗号ストア」）。
 *
 * 暗号ストアのファイル（OPFS）だけが消えると、`CryptoStore.open` は新しい端末を作るが、前の
 * `device_id` は暗号ストアと一緒に消えていて、前の端末がサーバーに残る。そこで、端末の ID を
 * 暗号ストアの外（IndexedDB `zoo-crypto-keys`。保存鍵を包む鍵と同じ置き場所）にも残し、新しい
 * 端末を作るときに前の端末を消せるようにする。`device_id` は秘密ではない。
 *
 * localStorage ではなく IndexedDB に置くのは、OPFS と同じ保存先の区切り（オリジンとパーティション）
 * になり、別の保存先（Safari のタブとホーム画面のアプリなど）の、今も使っている端末を前の端末と
 * 取り違えないため。Worker からも読み書きできる。
 */
export interface DeviceRecord {
  userId: string;
  deviceId: string;
  /** サーバーへの登録が済んだか（済んでいなければ、削除 API の 404 をすぐには済んだものとしない）。 */
  registered: boolean;
}

/** 端末の記録の置き場所。テストではメモリーにする。 */
export interface DeviceRecordStore {
  get(name: string): Promise<DeviceRecord | undefined>;
  put(name: string, record: DeviceRecord): Promise<void>;
  delete(name: string): Promise<void>;
}

const KEY_PREFIX = "device:";

function parse(value: unknown): DeviceRecord | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const { userId, deviceId, registered } = value as Partial<DeviceRecord>;
  if (typeof userId !== "string" || typeof deviceId !== "string") return undefined;
  return { userId, deviceId, registered: registered === true };
}

/** IndexedDB `zoo-crypto-keys` の置き場所（包む鍵と同じ object store に、キーを分けて置く）。 */
export const indexedDbDeviceRecords: DeviceRecordStore = {
  get: async (name) =>
    parse(await withStore("readonly", (store) => store.get(`${KEY_PREFIX}${name}`))),
  put: async (name, record) => {
    await withStore("readwrite", (store) => store.put({ ...record }, `${KEY_PREFIX}${name}`));
  },
  delete: async (name) => {
    await withStore("readwrite", (store) => store.delete(`${KEY_PREFIX}${name}`));
  },
};

/**
 * 記録を読む。読めなければ `undefined`（記録は前の端末を消すための補助で、読めなくても暗号ストアは
 * 開く。前の端末は、これまでどおり設定の「端末」から消せる）。
 */
export async function readDeviceRecord(
  records: DeviceRecordStore | undefined,
  name: string,
): Promise<DeviceRecord | undefined> {
  if (!records) return undefined;
  try {
    return parse(await records.get(name));
  } catch {
    return undefined;
  }
}

/** 記録を書く。失敗しても投げない（次に開いたときに書き直す）。 */
export async function writeDeviceRecord(
  records: DeviceRecordStore | undefined,
  name: string,
  record: DeviceRecord,
): Promise<void> {
  if (!records) return;
  try {
    await records.put(name, record);
  } catch {
    // 次に開いたとき（`CryptoStore.open`）に書き直す。
  }
}

/** 記録を消す（ログアウト）。失敗しても投げない。 */
export async function deleteDeviceRecord(
  records: DeviceRecordStore | undefined,
  name: string,
): Promise<void> {
  if (!records) return;
  try {
    await records.delete(name);
  } catch {
    // 残っても、次のログインで消すのは削除済みの端末（削除 API は冪等）。
  }
}
