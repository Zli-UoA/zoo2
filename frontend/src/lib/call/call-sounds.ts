/**
 * 通話の入退室効果音。音声ファイルは使わず Web Audio で短いチャイムを合成する。
 */

export type CallSoundKind = "join" | "leave" | "mute" | "unmute" | "ring" | "ringback";

export const CALL_SOUNDS_STORAGE_KEY = "zoo-call-sounds";
const CALL_SOUNDS_CHANGE_EVENT = "zoo-call-sounds-change";

// 保存に失敗したときだけ使う、現在のページ内の設定値
let memoryOverride: boolean | null = null;

export function isCallSoundsEnabled(): boolean {
  if (memoryOverride !== null) return memoryOverride;
  try {
    return window.localStorage.getItem(CALL_SOUNDS_STORAGE_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setCallSoundsEnabled(enabled: boolean): void {
  try {
    window.localStorage.setItem(CALL_SOUNDS_STORAGE_KEY, enabled ? "on" : "off");
    memoryOverride = null;
  } catch {
    // 保存できない環境でも、少なくとも現在のページでは選択を反映する
    memoryOverride = enabled;
  }
  try {
    window.dispatchEvent(new Event(CALL_SOUNDS_CHANGE_EVENT));
  } catch {}
}

/** 設定変更の購読 (useSyncExternalStore 用)。他タブの変更も拾う。 */
export function subscribeCallSounds(onChange: () => void): () => void {
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || e.key === CALL_SOUNDS_STORAGE_KEY) {
      memoryOverride = null;
      onChange();
    }
  };
  window.addEventListener(CALL_SOUNDS_CHANGE_EVENT, onChange);
  window.addEventListener("storage", onStorage);
  return () => {
    window.removeEventListener(CALL_SOUNDS_CHANGE_EVENT, onChange);
    window.removeEventListener("storage", onStorage);
  };
}

/**
 * 他ユーザーの入室で鳴らすか。自分のセッション、退出済みセッション、
 * 既に参加者一覧にいるセッション (重複通知・room_state 由来) では鳴らさない。
 */
export function shouldPlayRemoteJoin(
  sessionId: string,
  mySessionId: string,
  currentSessionIds: Iterable<string>,
  leftSessionIds: ReadonlySet<string>,
): boolean {
  if (sessionId === mySessionId) return false;
  if (leftSessionIds.has(sessionId)) return false;
  for (const id of currentSessionIds) {
    if (id === sessionId) return false;
  }
  return true;
}

/** 他ユーザーの退出で鳴らすか。参加者一覧に実在したセッションのみ。 */
export function shouldPlayRemoteLeave(
  sessionId: string,
  mySessionId: string,
  currentSessionIds: Iterable<string>,
): boolean {
  if (sessionId === mySessionId) return false;
  for (const id of currentSessionIds) {
    if (id === sessionId) return true;
  }
  return false;
}

let sharedContext: AudioContext | null = null;
let sinkId = "";

type SinkCapableContext = AudioContext & { setSinkId?: (id: string) => Promise<void> };

function applyContextSink(ctx: AudioContext): void {
  const c = ctx as SinkCapableContext;
  if (typeof c.setSinkId !== "function") return;
  // 空文字はシステム既定。非対応・失敗時は既定の出力のままにする
  void c.setSinkId(sinkId).catch(() => {});
}

/** 効果音の出力先 (選択中のスピーカー)。AudioContext.setSinkId 対応ブラウザのみ有効。 */
export function setCallSoundSink(deviceId: string): void {
  sinkId = deviceId;
  if (sharedContext && sharedContext.state !== "closed") applyContextSink(sharedContext);
}

function getContext(): AudioContext | null {
  if (sharedContext && sharedContext.state !== "closed") return sharedContext;
  try {
    const Ctor =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Ctor) return null;
    sharedContext = new Ctor();
    if (sinkId) applyContextSink(sharedContext);
    return sharedContext;
  } catch {
    return null;
  }
}

/** 自動再生制限を避けるため、ユーザー操作 (参加クリック) の中で呼ぶ。 */
export function primeCallSounds(): void {
  const ctx = getContext();
  if (ctx && ctx.state === "suspended") {
    void ctx.resume().catch(() => {});
  }
}

// join は上昇、leave は下降する 2 音のチャイム。mute / unmute は短く低めの 2 音で区別する
type SoundSpec = {
  notes: number[];
  step: number;
  length: number;
  /** 波形。省略は sine。 */
  wave?: OscillatorType;
  /** 音量 (ゲインのピーク)。省略は `PEAK_GAIN`。 */
  peak?: number;
};

const SOUNDS: Record<CallSoundKind, SoundSpec> = {
  join: { notes: [587.33, 880], step: 0.11, length: 0.18 },
  leave: { notes: [698.46, 440], step: 0.11, length: 0.18 },
  mute: { notes: [523.25, 349.23], step: 0.07, length: 0.12 },
  unmute: { notes: [349.23, 523.25], step: 0.07, length: 0.12 },
  // 着信音。気づかれやすいように、高低 2 音を 2 回続けて、大きめに鳴らす
  ring: {
    notes: [783.99, 659.25, 783.99, 659.25],
    step: 0.15,
    length: 0.2,
    wave: "triangle",
    peak: 0.5,
  },
  // 発信側の呼び出し音。相手を呼んでいる間、静かな 2 音を重ねて鳴らす（着信音とは別の音）
  ringback: { notes: [440, 480], step: 0, length: 0.9, peak: 0.14 },
};

// 音量 (ゲインのピーク)。Slack のように聞き取りやすい大きさにする
const PEAK_GAIN = 0.28;

/**
 * 効果音を 1 回鳴らす。鳴らした音を、途中で止める関数を返す（鳴らさなかったときは undefined）。
 * 止める対象は、この呼び出しで作った音源だけで、共有の AudioContext や、ほかの効果音には触れない。
 */
export function playCallSound(kind: CallSoundKind): (() => void) | undefined {
  if (!isCallSoundsEnabled()) return undefined;
  const ctx = getContext();
  if (!ctx) return undefined;
  try {
    if (ctx.state === "suspended") {
      void ctx.resume().catch(() => {});
      // 着信音は繰り返すので、止まっている間は積まない（再開した瞬間にまとめて鳴り、音が割れる）。
      if (kind === "ring" || kind === "ringback") return undefined;
    }
    const start = ctx.currentTime + 0.01;
    const { notes, step, length, wave = "sine", peak = PEAK_GAIN } = SOUNDS[kind];
    const nodes: { osc: OscillatorNode; gain: GainNode }[] = [];
    notes.forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      const t0 = start + i * step;
      osc.type = wave;
      osc.frequency.setValueAtTime(freq, t0);
      gain.gain.setValueAtTime(0.0001, t0);
      gain.gain.exponentialRampToValueAtTime(peak, t0 + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + length);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(t0);
      osc.stop(t0 + length + 0.02);
      nodes.push({ osc, gain });
    });
    return () => {
      for (const { osc, gain } of nodes) {
        try {
          // 予約済みの停止より前に、すぐ止める。すでに終わった音源は例外になるので、無視する
          osc.stop();
          gain.disconnect();
        } catch {
          // 止める失敗で通話を止めない
        }
      }
    };
  } catch {
    // 効果音の失敗で通話を止めない
    return undefined;
  }
}

/** 着信音を繰り返す間隔（ミリ秒）。鳴っている長さ（約 0.65 秒）を除くと、約 0.55 秒の間になる。 */
export const RINGTONE_INTERVAL_MS = 1200;

/** 発信側の呼び出し音を繰り返す間隔（ミリ秒）。0.9 秒鳴らして、2 秒あける。 */
export const RINGBACK_INTERVAL_MS = 2900;

/**
 * 着信音を鳴らし始め、止める関数を返す。効果音の設定（`isCallSoundsEnabled`）に従う。
 * ブラウザーの自動再生の制限で、ページを操作する前は鳴らないことがある。
 */
export function startRingtone(): () => void {
  playCallSound("ring");
  const timer = setInterval(() => playCallSound("ring"), RINGTONE_INTERVAL_MS);
  return () => clearInterval(timer);
}

/**
 * 発信側の呼び出し音。自分が呼んでいる相手が出るまで、静かな音を繰り返し、止める関数を返す。
 * 着信音と同じく、効果音の設定（`isCallSoundsEnabled`）に従う。
 * 止める関数は、繰り返しだけでなく、いま鳴っている音も止める。
 */
export function startRingback(): () => void {
  let stopSound = playCallSound("ringback");
  const timer = setInterval(() => {
    stopSound = playCallSound("ringback");
  }, RINGBACK_INTERVAL_MS);
  return () => {
    clearInterval(timer);
    // 鳴っている途中の音も止める（応答の直後に、会話へ重ならないようにする）
    stopSound?.();
  };
}
