import type { SfuAdapter } from "./types";

/**
 * SFU のアダプタへ渡すマイクを、1 件ずつ今のマイクにそろえる。
 *
 * マイクの切替と、接続後のそろえ直しが別々に `replaceMicTrack` を呼ぶと、古いマイクの差し替えが
 * 新しいマイクの差し替えより後に終わり、止めたマイクを送り続けることがある。どちらもこのキューを通し、
 * 送るマイクは予約したときではなく実行するときに `desiredMic()` から読む。
 */
export interface SfuMicSync {
  /** アダプタを作ったときに渡したマイクを記録する。 */
  created(adapter: SfuAdapter, track: MediaStreamTrack): void;
  /**
   * キューの順番が来たら、`adapter` が今も使われていれば、その時点のマイクにそろえる。
   * 送ったマイク（そろえる必要がなかった場合も含む）を返し、対象外なら null を返す。
   * 差し替えの失敗はそのまま返す（キューは止めない）。
   */
  sync(adapter: SfuAdapter): Promise<MediaStreamTrack | null>;
  /**
   * アダプタを閉じた。閉じたアダプタの差し替えは解決しないまま残ることがあるので、
   * 待っている処理と予約済みの処理を null で終え、キューを次へ進める。
   */
  closed(adapter: SfuAdapter): void;
}

export function createSfuMicSync(options: {
  /** 今使っているアダプタ。閉じたアダプタには触れない。 */
  currentAdapter: () => SfuAdapter | null;
  /** 今送るべきマイク（切替中なら切替先）。 */
  desiredMic: () => MediaStreamTrack | null;
}): SfuMicSync {
  let queue: Promise<unknown> = Promise.resolve();
  const sent = new WeakMap<SfuAdapter, MediaStreamTrack>();
  // アダプタごとの「閉じた」合図。閉じたら解決する
  const closeSignals = new WeakMap<SfuAdapter, { promise: Promise<null>; release: () => void }>();
  const closeSignal = (adapter: SfuAdapter) => {
    let signal = closeSignals.get(adapter);
    if (!signal) {
      let release = () => {};
      const promise = new Promise<null>((resolve) => {
        release = () => resolve(null);
      });
      signal = { promise, release };
      closeSignals.set(adapter, signal);
    }
    return signal;
  };
  return {
    created(adapter, track) {
      sent.set(adapter, track);
    },
    sync(adapter) {
      const step = queue.then(async () => {
        if (options.currentAdapter() !== adapter) return null;
        const mic = options.desiredMic();
        if (!mic) return null;
        if (sent.get(adapter) === mic) return mic;
        await adapter.replaceMicTrack(mic);
        sent.set(adapter, mic);
        return mic;
      });
      // 前の処理や自分の差し替えが閉じたアダプタで止まっても、閉じた時点で終える。
      // 同じアダプタへの処理は、閉じるまでは予約順に 1 件ずつ実行される
      const run = Promise.race([step, closeSignal(adapter).promise]);
      queue = run.catch(() => {});
      return run;
    },
    closed(adapter) {
      closeSignal(adapter).release();
    },
  };
}
