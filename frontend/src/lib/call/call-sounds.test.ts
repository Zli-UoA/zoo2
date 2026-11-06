// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vite-plus/test";

import {
  CALL_SOUNDS_STORAGE_KEY,
  isCallSoundsEnabled,
  setCallSoundsEnabled,
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
});
