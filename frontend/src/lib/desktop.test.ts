import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  DESKTOP_PROTOCOL_VERSION,
  desktopAccessToken,
  detectDesktop,
  hasDesktopBridge,
  isDesktopSession,
  resetDesktopForTest,
} from "./desktop";

type Internals = { invoke: ReturnType<typeof vi.fn>; transformCallback: ReturnType<typeof vi.fn> };

function install(handler: (cmd: string) => unknown): Internals {
  const internals = {
    invoke: vi.fn(async (cmd: string) => handler(cmd)),
    transformCallback: vi.fn(() => 1),
  };
  // unit テストは node で動くので、Tauri の WebView の window を差し込む。
  vi.stubGlobal("window", { __TAURI_INTERNALS__: internals });
  return internals;
}

afterEach(() => {
  vi.unstubAllGlobals();
  resetDesktopForTest();
  vi.useRealTimers();
});

const session = (protocol: number, ui: string) => ({
  protocol,
  ui,
  api_base_url: "https://zli.chat",
  state: { state: "ready" },
});

describe("detectDesktop", () => {
  it("is false in a normal browser", async () => {
    expect(hasDesktopBridge()).toBe(false);
    await expect(detectDesktop()).resolves.toBe(false);
  });

  it("accepts only the same protocol in remote mode", async () => {
    install(() => session(DESKTOP_PROTOCOL_VERSION, "remote"));
    await expect(detectDesktop()).resolves.toBe(true);
    expect(isDesktopSession()).toBe(true);
  });

  it("falls back to the web for another protocol or the bundled UI", async () => {
    install(() => session(DESKTOP_PROTOCOL_VERSION + 1, "remote"));
    await expect(detectDesktop()).resolves.toBe(false);
    resetDesktopForTest();
    install(() => session(DESKTOP_PROTOCOL_VERSION, "leptos"));
    await expect(detectDesktop()).resolves.toBe(false);
  });
});

describe("desktopAccessToken", () => {
  it("reuses the token until a minute before it expires", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-11-11T00:00:00Z"));
    const now = Date.now() / 1000;
    const internals = install((cmd) =>
      cmd === "zoo_session"
        ? session(DESKTOP_PROTOCOL_VERSION, "remote")
        : { access_token: "t1", expires_at: now + 300 },
    );
    await detectDesktop();
    await expect(desktopAccessToken()).resolves.toBe("t1");
    await expect(desktopAccessToken()).resolves.toBe("t1");
    expect(internals.invoke).toHaveBeenCalledTimes(2);

    vi.setSystemTime(new Date((now + 241) * 1000));
    await desktopAccessToken();
    expect(internals.invoke).toHaveBeenCalledTimes(3);
  });

  it("returns null when the shell refuses", async () => {
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      throw { code: "not_logged_in", message: "x" };
    });
    await detectDesktop();
    await expect(desktopAccessToken()).resolves.toBeNull();
  });
});
