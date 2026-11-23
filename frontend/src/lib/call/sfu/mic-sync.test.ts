import { describe, expect, it } from "vite-plus/test";

import { createSfuMicSync } from "./mic-sync";
import type { SfuAdapter } from "./types";

interface FakeMic {
  id: string;
  stopped: boolean;
}

/** replaceMicTrack を手で完了させるアダプタ。完了した順に送るマイクが変わる。 */
function fakeAdapter(initial: FakeMic) {
  const pending: { track: FakeMic; resolve: () => void; reject: (e: Error) => void }[] = [];
  const state = { sending: initial, calls: [] as string[] };
  const adapter = {
    kind: "cloudflare",
    replaceMicTrack: (track: MediaStreamTrack) =>
      new Promise<void>((resolve, reject) => {
        const mic = track as unknown as FakeMic;
        state.calls.push(mic.id);
        pending.push({
          track: mic,
          resolve: () => {
            state.sending = mic;
            resolve();
          },
          reject,
        });
      }),
  } as unknown as SfuAdapter;
  const flush = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  };
  /** 待っている差し替えを 1 件完了させる。 */
  const finishNext = async (error?: Error) => {
    await flush();
    const next = pending.shift();
    if (!next) throw new Error("no pending replaceMicTrack");
    if (error) next.reject(error);
    else next.resolve();
    await flush();
  };
  return { adapter, state, finishNext, flush, pendingCount: () => pending.length };
}

/** call-context の使用中のマイクと、切替中の置換待ちのマイク。 */
function micState(initial: FakeMic) {
  return { current: initial, pending: null as FakeMic | null };
}

const mic = (id: string): FakeMic => ({ id, stopped: false });

describe("createSfuMicSync", () => {
  it("sends the latest mic when A→B→C switching overlaps the post-connect resync", async () => {
    const [a, b, c] = [mic("A"), mic("B"), mic("C")];
    const mics = micState(a);
    const { adapter, state, finishNext, flush, pendingCount } = fakeAdapter(a);
    let current: SfuAdapter | null = adapter;
    const sync = createSfuMicSync({
      currentAdapter: () => current,
      desiredMic: () => (mics.pending ?? mics.current) as unknown as MediaStreamTrack,
    });
    // 接続待ちのアダプタを A で作る
    sync.created(adapter, a as unknown as MediaStreamTrack);
    // call-context の切替と同じ順で、置換待ち → SFU の差し替え → 使用中の更新 → 旧マイクの停止を行う
    const switchTo = async (next: FakeMic) => {
      mics.pending = next;
      try {
        await sync.sync(adapter);
      } finally {
        mics.pending = null;
      }
      const old = mics.current;
      mics.current = next;
      // SFU が旧マイクを送っていないことを確かめてから止める
      expect(state.sending).not.toBe(old);
      old.stopped = true;
    };

    // 接続待ちのあいだに A→B を切り替える
    const toB = switchTo(b);
    await finishNext();
    await toB;
    expect(a.stopped).toBe(true);

    // B→C の差し替え中に接続が完了し、接続後のそろえ直しが予約される
    const toC = switchTo(c);
    await flush();
    expect(pendingCount()).toBe(1);
    const resync = sync.sync(adapter);
    await finishNext();
    await toC;
    await expect(resync).resolves.toBe(c);
    await flush();

    // そろえ直しは順番が来た時点の C を読むので、B で上書きしない
    expect(pendingCount()).toBe(0);
    expect(state.calls).toEqual(["B", "C"]);
    expect(state.sending).toBe(c);
    expect(c.stopped).toBe(false);
    expect(b.stopped).toBe(true);
    current = null;
  });

  it("reads the mic when its turn comes, and skips closed adapters", async () => {
    const [a, b, c] = [mic("A"), mic("B"), mic("C")];
    const mics = micState(a);
    const { adapter, state, finishNext } = fakeAdapter(a);
    let current: SfuAdapter | null = adapter;
    const sync = createSfuMicSync({
      currentAdapter: () => current,
      desiredMic: () => (mics.pending ?? mics.current) as unknown as MediaStreamTrack,
    });
    sync.created(adapter, a as unknown as MediaStreamTrack);

    // 予約したときは B でも、実行する前に C へ変わっていれば C を送る
    mics.current = b;
    const first = sync.sync(adapter);
    const second = sync.sync(adapter);
    mics.current = c;
    await finishNext();
    await expect(first).resolves.toBe(c);
    await expect(second).resolves.toBe(c);
    expect(state.calls).toEqual(["C"]);

    // 失敗してもキューは止まらず、閉じたアダプタには触れない
    mics.current = b;
    const failed = sync.sync(adapter);
    await finishNext(new Error("boom"));
    await expect(failed).rejects.toThrow("boom");
    current = null;
    await expect(sync.sync(adapter)).resolves.toBeNull();
    expect(state.calls).toEqual(["C", "B"]);
  });
});
