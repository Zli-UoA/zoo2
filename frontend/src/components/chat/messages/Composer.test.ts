import { createI18n } from "../../../lib/i18n";
import { describe, expect, it } from "vite-plus/test";

import {
  canSubmitComposer,
  composerSendHint,
  resolveComposerPlatform,
  shouldSendFromKey,
  type ComposerKeyInput,
  type ComposerPlatform,
} from "./Composer";

const ALL_PLATFORMS: ComposerPlatform[] = ["mac", "windows", "linux", "unknown"];

function keyInput(overrides: Partial<ComposerKeyInput> = {}): ComposerKeyInput {
  return {
    key: "Enter",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    isComposing: false,
    ...overrides,
  };
}

describe("shouldSendFromKey", () => {
  it("macOS は Cmd+Enter で送信する", () => {
    expect(shouldSendFromKey(keyInput({ metaKey: true }), "mac")).toBe(true);
  });

  it("macOS の Ctrl+Enter では送信しない", () => {
    expect(shouldSendFromKey(keyInput({ ctrlKey: true }), "mac")).toBe(false);
  });

  it("Windows / Linux は Ctrl+Enter で送信する", () => {
    expect(shouldSendFromKey(keyInput({ ctrlKey: true }), "windows")).toBe(true);
    expect(shouldSendFromKey(keyInput({ ctrlKey: true }), "linux")).toBe(true);
  });

  it("Windows / Linux の Cmd+Enter では送信しない", () => {
    expect(shouldSendFromKey(keyInput({ metaKey: true }), "windows")).toBe(false);
    expect(shouldSendFromKey(keyInput({ metaKey: true }), "linux")).toBe(false);
  });

  it("環境を判別できないときは Cmd / Ctrl のどちらでも送信する", () => {
    expect(shouldSendFromKey(keyInput({ metaKey: true }), "unknown")).toBe(true);
    expect(shouldSendFromKey(keyInput({ ctrlKey: true }), "unknown")).toBe(true);
  });

  it("Enter 単独はどの環境でも改行にする", () => {
    for (const platform of ALL_PLATFORMS) {
      expect(shouldSendFromKey(keyInput(), platform)).toBe(false);
    }
  });

  it("Shift+Enter は修飾キーが付いていても改行にする", () => {
    expect(shouldSendFromKey(keyInput({ shiftKey: true, metaKey: true }), "mac")).toBe(false);
    expect(shouldSendFromKey(keyInput({ shiftKey: true, ctrlKey: true }), "windows")).toBe(false);
    expect(shouldSendFromKey(keyInput({ shiftKey: true, ctrlKey: true }), "linux")).toBe(false);
    expect(shouldSendFromKey(keyInput({ shiftKey: true }), "unknown")).toBe(false);
  });

  it("IME 変換中（isComposing）は送信キーでも送信しない", () => {
    expect(shouldSendFromKey(keyInput({ isComposing: true, metaKey: true }), "mac")).toBe(false);
    expect(shouldSendFromKey(keyInput({ isComposing: true, ctrlKey: true }), "windows")).toBe(
      false,
    );
    expect(shouldSendFromKey(keyInput({ isComposing: true, ctrlKey: true }), "linux")).toBe(false);
    expect(shouldSendFromKey(keyInput({ isComposing: true }), "unknown")).toBe(false);
  });

  it("Enter 以外のキーや修飾なし Enter では送信しない", () => {
    expect(shouldSendFromKey(keyInput({ key: "a", metaKey: true }), "mac")).toBe(false);
    expect(shouldSendFromKey(keyInput({ key: "a", ctrlKey: true }), "windows")).toBe(false);
    expect(shouldSendFromKey(keyInput({ key: "Escape" }), "unknown")).toBe(false);
  });
});

describe("canSubmitComposer", () => {
  it("空白のみ・空文字は送信しない", () => {
    expect(canSubmitComposer("", false)).toBe(false);
    expect(canSubmitComposer("   ", false)).toBe(false);
    expect(canSubmitComposer("\n", false)).toBe(false);
  });

  it("内容があれば送信できる", () => {
    expect(canSubmitComposer("こんにちは", false)).toBe(true);
    expect(canSubmitComposer("  hi  ", false)).toBe(true);
  });

  it("入力無効時は送信しない", () => {
    expect(canSubmitComposer("こんにちは", true)).toBe(false);
  });
});

describe("resolveComposerPlatform", () => {
  it("macOS を判定する", () => {
    expect(resolveComposerPlatform("MacIntel")).toBe("mac");
    expect(resolveComposerPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe("mac");
  });

  it("Windows を判定する", () => {
    expect(resolveComposerPlatform("Win32")).toBe("windows");
    expect(resolveComposerPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
  });

  it("Linux を判定する", () => {
    expect(resolveComposerPlatform("Linux x86_64")).toBe("linux");
    expect(resolveComposerPlatform("X11")).toBe("linux");
  });

  it("判別できないときは unknown にする", () => {
    expect(resolveComposerPlatform("")).toBe("unknown");
    expect(resolveComposerPlatform("iPhone")).toBe("unknown");
  });
});

describe("composerSendHint", () => {
  const t = createI18n().t;
  it("macOS は ⌘（Cmd）+Enter を案内する", () => {
    expect(composerSendHint("mac", t)).toBe("⌘（Cmd）+Enter で送信 / Enter で改行");
  });

  it("Windows / Linux は Ctrl+Enter を案内する", () => {
    expect(composerSendHint("windows", t)).toBe("Ctrl+Enter で送信 / Enter で改行");
    expect(composerSendHint("linux", t)).toBe("Ctrl+Enter で送信 / Enter で改行");
  });

  it("判別できないときは Cmd / Ctrl の両方を案内する", () => {
    expect(composerSendHint("unknown", t)).toBe("Cmd / Ctrl+Enter で送信 / Enter で改行");
  });
});
