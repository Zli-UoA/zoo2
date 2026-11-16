// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import {
  CALL_SOUNDS_STORAGE_KEY,
  isCallSoundsEnabled,
  RINGBACK_INTERVAL_MS,
  RINGTONE_INTERVAL_MS,
  setCallSoundsEnabled,
  startRingback,
  startRingtone,
  shouldPlayRemoteJoin,
  shouldPlayRemoteLeave,
} from "./call-sounds";

describe("shouldPlayRemoteJoin", () => {
  const none = new Set<string>();
  it("鳴らす: 未登録の他セッション", () => {
    expect(shouldPlayRemoteJoin("s2", "s1", ["s1"], none)).toBe(true);
  });
  it("自分のセッションでは鳴らさない", () => {
    expect(shouldPlayRemoteJoin("s1", "s1", [], none)).toBe(false);
  });
  it("既に参加者にいる (重複・room_state 由来) 場合は鳴らさない", () => {
    expect(shouldPlayRemoteJoin("s2", "s1", ["s1", "s2"], none)).toBe(false);
  });
  it("退出済みセッションでは鳴らさない", () => {
    expect(shouldPlayRemoteJoin("s2", "s1", ["s1"], new Set(["s2"]))).toBe(false);
  });
  it("同一ユーザーの別セッションでも鳴らす", () => {
    expect(shouldPlayRemoteJoin("s3", "s1", ["s1", "s2"], none)).toBe(true);
  });
});

describe("shouldPlayRemoteLeave", () => {
  it("参加者にいた他セッションの退出で鳴らす", () => {
    expect(shouldPlayRemoteLeave("s2", "s1", ["s1", "s2"])).toBe(true);
  });
  it("参加者にいないセッションでは鳴らさない", () => {
    expect(shouldPlayRemoteLeave("s9", "s1", ["s1", "s2"])).toBe(false);
  });
  it("自分では鳴らさない", () => {
    expect(shouldPlayRemoteLeave("s1", "s1", ["s1"])).toBe(false);
  });
});

describe("同一チャンクで join→leave が続く場合", () => {
  it("同期的に更新した集合なら退出音も鳴る", () => {
    const known = new Set<string>(["s1"]);
    expect(shouldPlayRemoteJoin("s2", "s1", known, new Set())).toBe(true);
    known.add("s2");
    expect(shouldPlayRemoteLeave("s2", "s1", known)).toBe(true);
    known.delete("s2");
    // 退出後の重複通知では鳴らさない
    expect(shouldPlayRemoteLeave("s2", "s1", known)).toBe(false);
  });
});

describe("call sounds setting", () => {
  beforeEach(() => window.localStorage.clear());
  it("既定はオン", () => {
    expect(isCallSoundsEnabled()).toBe(true);
  });
  it("オフを保存できる", () => {
    setCallSoundsEnabled(false);
    expect(window.localStorage.getItem(CALL_SOUNDS_STORAGE_KEY)).toBe("off");
    expect(isCallSoundsEnabled()).toBe(false);
    setCallSoundsEnabled(true);
    expect(isCallSoundsEnabled()).toBe(true);
  });
  describe("保存に失敗する環境", () => {
    afterEach(() => {
      vi.restoreAllMocks();
      // 成功する保存で現在ページ内の値を初期化する
      setCallSoundsEnabled(true);
    });
    it("保存できなくても現在のページでは選択を反映する", () => {
      vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
        throw new Error("QuotaExceededError");
      });
      setCallSoundsEnabled(false);
      expect(isCallSoundsEnabled()).toBe(false);
      setCallSoundsEnabled(true);
      expect(isCallSoundsEnabled()).toBe(true);
    });
  });
});

// `call-sounds.ts` は AudioContext を使い回すので、偽物はテストをまたいで同じものを使い、数だけ数え直す。
const oscillators: unknown[] = [];
class FakeContext {
  state = "running";
  currentTime = 0;
  destination = {};
  createOscillator() {
    const osc = {
      type: "",
      frequency: { setValueAtTime() {} },
      connect() {},
      start() {},
      stop() {},
    };
    oscillators.push(osc);
    return osc;
  }
  createGain() {
    return { gain: { setValueAtTime() {}, exponentialRampToValueAtTime() {} }, connect() {} };
  }
}

describe("startRingtone", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("すぐに鳴らし、止めるまで一定の間隔で繰り返す", () => {
    vi.useFakeTimers();
    oscillators.length = 0;
    vi.stubGlobal("AudioContext", FakeContext);
    setCallSoundsEnabled(true);

    const stop = startRingtone();
    expect(oscillators.length).toBe(4);
    vi.advanceTimersByTime(RINGTONE_INTERVAL_MS);
    expect(oscillators.length).toBe(8);
    stop();
    vi.advanceTimersByTime(RINGTONE_INTERVAL_MS * 3);
    expect(oscillators.length).toBe(8);
  });

  it("着信音は、鳴り終わる前に次が始まらない間隔で、2 秒より短く繰り返す", () => {
    expect(RINGTONE_INTERVAL_MS).toBeLessThan(2000);
    // 4 音（0.15 秒ずつずらして、最後は 0.2 秒）が鳴り終わる長さ
    expect(RINGTONE_INTERVAL_MS / 1000).toBeGreaterThan(0.15 * 3 + 0.2);
  });
});

describe("startRingback", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("すぐに鳴らし、止めるまで繰り返す。設定で切ると鳴らさない", () => {
    vi.useFakeTimers();
    oscillators.length = 0;
    vi.stubGlobal("AudioContext", FakeContext);
    setCallSoundsEnabled(true);

    const stop = startRingback();
    expect(oscillators.length).toBe(2);
    vi.advanceTimersByTime(RINGBACK_INTERVAL_MS);
    expect(oscillators.length).toBe(4);
    stop();
    vi.advanceTimersByTime(RINGBACK_INTERVAL_MS * 3);
    expect(oscillators.length).toBe(4);

    setCallSoundsEnabled(false);
    const stopOff = startRingback();
    vi.advanceTimersByTime(RINGBACK_INTERVAL_MS);
    stopOff();
    expect(oscillators.length).toBe(4);
    setCallSoundsEnabled(true);
  });
});
