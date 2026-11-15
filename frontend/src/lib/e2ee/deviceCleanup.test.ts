import { describe, expect, it } from "vite-plus/test";

import { cleanupReasons } from "./deviceCleanup";

const NOW = Date.parse("2026-11-15T00:00:00Z");

function device(overrides: Partial<Parameters<typeof cleanupReasons>[0]> = {}) {
  return {
    createdAt: "2026-11-01T00:00:00Z",
    lastSeenAt: "2026-11-14T00:00:00Z",
    current: false,
    signed: true,
    ...overrides,
  };
}

describe("cleanupReasons", () => {
  it("最近使った、解錠済みの端末は勧めない", () => {
    expect(cleanupReasons(device(), { now: NOW, anySigned: true })).toEqual([]);
  });

  it("30 日以上使っていない端末を勧める", () => {
    expect(
      cleanupReasons(device({ lastSeenAt: "2026-10-16T00:00:00Z" }), {
        now: NOW,
        anySigned: true,
      }),
    ).toEqual(["stale"]);
    expect(
      cleanupReasons(device({ lastSeenAt: "2026-10-17T00:00:00Z" }), {
        now: NOW,
        anySigned: true,
      }),
    ).toEqual([]);
  });

  it("使った日時がなければ、登録日時で数える", () => {
    expect(
      cleanupReasons(device({ createdAt: "2026-09-01T00:00:00Z", lastSeenAt: null }), {
        now: NOW,
        anySigned: true,
      }),
    ).toEqual(["stale"]);
  });

  it("解錠していないまま 7 日以上使っていない端末を勧める", () => {
    expect(
      cleanupReasons(
        device({
          signed: false,
          createdAt: "2026-11-01T00:00:00Z",
          lastSeenAt: "2026-11-08T00:00:00Z",
        }),
        { now: NOW, anySigned: true },
      ),
    ).toEqual(["notUnlocked"]);
  });

  it("署名がなくても、最近使った端末は勧めない（失効のあと、解錠し直す前の端末など）", () => {
    // 登録から日がたっていても、1 時間前に使った。
    expect(
      cleanupReasons(
        device({
          signed: false,
          createdAt: "2026-10-01T00:00:00Z",
          lastSeenAt: "2026-11-14T23:00:00Z",
        }),
        { now: NOW, anySigned: true },
      ),
    ).toEqual([]);
    // 登録したばかり（解錠の途中かもしれない）も勧めない。
    expect(
      cleanupReasons(
        device({ signed: false, createdAt: "2026-11-14T12:00:00Z", lastSeenAt: null }),
        { now: NOW, anySigned: true },
      ),
    ).toEqual([]);
  });

  it("署名のある端末が 1 台もなければ、解錠していないことでは勧めない", () => {
    expect(cleanupReasons(device({ signed: false }), { now: NOW, anySigned: false })).toEqual([]);
  });

  it("今の端末は勧めない", () => {
    expect(
      cleanupReasons(device({ current: true, signed: false, lastSeenAt: "2026-01-01T00:00:00Z" }), {
        now: NOW,
        anySigned: true,
      }),
    ).toEqual([]);
  });
});
