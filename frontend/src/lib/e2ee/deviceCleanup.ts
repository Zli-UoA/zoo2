/**
 * 設定の「端末」で、削除を勧める端末（docs/e2ee.md の「ログアウトと端末の削除」）。自動では消さない。
 *
 * 端末の削除は失効ではないので、勧めるだけにして、消すかは利用者が決める。
 */

/** 長く使っていないとみなす日数（20 台の上限の `stale_devices` と同じ 30 日）。 */
export const STALE_DEVICE_DAYS = 30;
/** 登録してから、解錠していない端末として勧めるまでの時間（解錠の途中の端末を勧めない）。 */
const NOT_UNLOCKED_GRACE_MS = 24 * 60 * 60 * 1000;
const DAY_MS = 24 * 60 * 60 * 1000;

export type CleanupReason = "stale" | "notUnlocked";

export interface CleanupDevice {
  createdAt: string;
  lastSeenAt: string | null;
  current: boolean;
  signed: boolean;
}

function timeOf(value: string | null): number | null {
  if (!value) return null;
  const time = Date.parse(value);
  return Number.isNaN(time) ? null : time;
}

/**
 * 削除を勧める理由（なければ空）。今の端末は勧めない。
 *
 * - `stale`：最後に使ってから（なければ登録から）{@link STALE_DEVICE_DAYS} 日以上たった。
 * - `notUnlocked`：自己署名がない（解錠していない）まま、登録から 1 日以上たった。自己署名のある端末が
 *   1 台もない（クロス署名鍵がまだない）ときは、どの端末にも署名がないので勧めない（`anySigned`）。
 */
export function cleanupReasons(
  device: CleanupDevice,
  { now, anySigned }: { now: number; anySigned: boolean },
): CleanupReason[] {
  if (device.current) return [];
  const reasons: CleanupReason[] = [];
  const created = timeOf(device.createdAt);
  const used = timeOf(device.lastSeenAt) ?? created;
  if (used !== null && now - used >= STALE_DEVICE_DAYS * DAY_MS) reasons.push("stale");
  if (anySigned && !device.signed && created !== null && now - created >= NOT_UNLOCKED_GRACE_MS) {
    reasons.push("notUnlocked");
  }
  return reasons;
}
