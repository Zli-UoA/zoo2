/**
 * 通話の入退室効果音。音声ファイルは使わず Web Audio で短いチャイムを合成する。
 */

export type CallSoundKind = "join" | "leave";

export const CALL_SOUNDS_STORAGE_KEY = "zoo-call-sounds";
const CALL_SOUNDS_CHANGE_EVENT = "zoo-call-sounds-change";

export function isCallSoundsEnabled(): boolean {
  try {
    return window.localStorage.getItem(CALL_SOUNDS_STORAGE_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setCallSoundsEnabled(enabled: boolean): void {
  try {
    window.localStorage.setItem(CALL_SOUNDS_STORAGE_KEY, enabled ? "on" : "off");
  } catch {
    // ストレージが使えない環境では保存しない
  }
  try {
    window.dispatchEvent(new Event(CALL_SOUNDS_CHANGE_EVENT));
  } catch {}
}

/** 設定変更の購読 (useSyncExternalStore 用)。他タブの変更も拾う。 */
export function subscribeCallSounds(onChange: () => void): () => void {
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || e.key === CALL_SOUNDS_STORAGE_KEY) onChange();
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

// join は上昇、leave は下降する 2 音のチャイム
const NOTES: Record<CallSoundKind, [number, number]> = {
  join: [587.33, 880],
  leave: [698.46, 440],
};

export function playCallSound(kind: CallSoundKind): void {
  if (!isCallSoundsEnabled()) return;
  const ctx = getContext();
  if (!ctx) return;
  try {
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    const start = ctx.currentTime + 0.01;
    const step = 0.11;
    NOTES[kind].forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      const t0 = start + i * step;
      osc.type = "sine";
      osc.frequency.setValueAtTime(freq, t0);
      gain.gain.setValueAtTime(0.0001, t0);
      gain.gain.exponentialRampToValueAtTime(0.12, t0 + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + 0.18);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(t0);
      osc.stop(t0 + 0.2);
    });
  } catch {
    // 効果音の失敗で通話を止めない
  }
}
