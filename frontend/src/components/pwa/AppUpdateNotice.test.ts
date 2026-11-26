import { describe, expect, it } from "vite-plus/test";

import { isReloadShortcut } from "./AppUpdateNotice";

const key = (init: {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
}) => ({
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  ...init,
});

describe("isReloadShortcut", () => {
  it("F5、Ctrl+R、⌘R は再読み込み", () => {
    expect(isReloadShortcut(key({ key: "F5" }))).toBe(true);
    expect(isReloadShortcut(key({ key: "r", ctrlKey: true }))).toBe(true);
    expect(isReloadShortcut(key({ key: "R", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isReloadShortcut(key({ key: "r", metaKey: true }))).toBe(true);
  });

  it("修飾キーのない R や、Alt との組み合わせは再読み込みではない", () => {
    expect(isReloadShortcut(key({ key: "r" }))).toBe(false);
    expect(isReloadShortcut(key({ key: "r", ctrlKey: true, altKey: true }))).toBe(false);
    expect(isReloadShortcut(key({ key: "e", ctrlKey: true }))).toBe(false);
  });
});
