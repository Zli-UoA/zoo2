import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  DESKTOP_PROTOCOL_VERSION,
  desktopAccessToken,
  desktopDownloadBegin,
  desktopDownloadCancel,
  desktopDownloadFinish,
  desktopDownloadSettings,
  desktopDownloadWrite,
  desktopNotificationSettings,
  desktopPickDownloadDirectory,
  desktopSetBadge,
  desktopSetDownloadSettings,
  desktopSetNotifications,
  desktopTakeCallAccept,
  detectDesktop,
  hasDesktopBridge,
  isDesktopSession,
  resetDesktopForTest,
} from "./desktop";
import type { AppConfig } from "./config";
import { ReauthError, startReauth } from "./e2ee/reauth";

type Internals = { invoke: ReturnType<typeof vi.fn>; transformCallback: ReturnType<typeof vi.fn> };

function install(handler: (cmd: string, args?: unknown, options?: unknown) => unknown): Internals {
  const internals = {
    invoke: vi.fn(async (cmd: string, args?: unknown, options?: unknown) =>
      handler(cmd, args, options),
    ),
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

describe("desktopNotificationSettings", () => {
  it("reads and writes the app's notification setting", async () => {
    const internals = install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      if (cmd === "zoo_notification_settings") return { enabled: false };
      return undefined;
    });
    await detectDesktop();
    await expect(desktopNotificationSettings()).resolves.toEqual({ enabled: false });
    await desktopSetNotifications(true);
    expect(internals.invoke).toHaveBeenCalledWith("zoo_set_notifications", {
      settings: { enabled: true },
    });
  });

  it("is null when an older app does not have the command", async () => {
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      throw new Error("command zoo_notification_settings not found");
    });
    await detectDesktop();
    await expect(desktopNotificationSettings()).resolves.toBeNull();
  });
});

describe("download commands", () => {
  it("reads and writes the save location used by the app", async () => {
    const internals = install((cmd, args) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      if (cmd === "zoo_download_settings") return { ask_every_time: true, directory: null };
      if (cmd === "zoo_set_download_settings")
        return (args as { settings?: unknown }).settings ?? null;
      if (cmd === "zoo_pick_download_directory") return "C:\\Users\\zoo\\Downloads";
      return undefined;
    });
    await detectDesktop();

    await expect(desktopDownloadSettings()).resolves.toEqual({
      ask_every_time: true,
      directory: null,
    });
    const next = { ask_every_time: false, directory: "C:\\Users\\zoo\\Documents" };
    await expect(desktopSetDownloadSettings(next)).resolves.toEqual(next);
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_set_download_settings", {
      settings: next,
    });
    await expect(desktopPickDownloadDirectory()).resolves.toBe("C:\\Users\\zoo\\Downloads");
  });

  it("passes raw bytes and the download id in a header", async () => {
    const internals = install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      if (cmd === "zoo_download_begin")
        return { status: "started", id: "d1", path: "C:\\a\\b.pdf" };
      return null;
    });
    await detectDesktop();

    await expect(desktopDownloadBegin("b.pdf")).resolves.toEqual({
      status: "started",
      id: "d1",
      path: "C:\\a\\b.pdf",
    });
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_download_begin", {
      begin: { file_name: "b.pdf" },
    });

    const bytes = new Uint8Array([1, 2, 3]);
    await desktopDownloadWrite("d1", bytes);
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_download_write", bytes, {
      headers: { "x-zoo-download-id": "d1" },
    });

    await desktopDownloadFinish("d1");
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_download_finish", { id: "d1" });
    await desktopDownloadCancel("d1");
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_download_cancel", { id: "d1" });
  });

  it("is null when an older app does not have the download commands", async () => {
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      throw new Error(`command ${cmd} not found`);
    });
    await detectDesktop();
    await expect(desktopDownloadSettings()).resolves.toBeNull();
  });
});

describe("desktopTakeCallAccept", () => {
  it("returns the invite id the toast's accept button left, and null otherwise", async () => {
    let next: unknown = "00000000-0000-4000-8000-000000000001";
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      if (cmd === "zoo_take_call_accept") return next;
      return undefined;
    });
    await detectDesktop();
    await expect(desktopTakeCallAccept()).resolves.toBe("00000000-0000-4000-8000-000000000001");
    next = null;
    await expect(desktopTakeCallAccept()).resolves.toBeNull();
  });

  it("is null when an older app does not have the command", async () => {
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      throw new Error("command zoo_take_call_accept not found");
    });
    await detectDesktop();
    await expect(desktopTakeCallAccept()).resolves.toBeNull();
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

  it("throws on a transient failure so the request is retried instead of logging out", async () => {
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      throw { code: "network", message: "IdP が応答しません" };
    });
    await detectDesktop();
    await expect(desktopAccessToken()).rejects.toThrow("IdP が応答しません");
  });

  it("returns null in dev mode so requests go out with X-Dev-User", async () => {
    install((cmd) => {
      if (cmd === "zoo_session") return session(DESKTOP_PROTOCOL_VERSION, "remote");
      throw { code: "wrong_mode", message: "dev モードではトークンがありません" };
    });
    await detectDesktop();
    await expect(desktopAccessToken()).resolves.toBeNull();
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

describe("startReauth on the desktop app", () => {
  const config = {} as AppConfig;

  async function desktop(reauth: () => unknown) {
    const internals = install((cmd) =>
      cmd === "zoo_session" ? session(DESKTOP_PROTOCOL_VERSION, "remote") : reauth(),
    );
    await detectDesktop();
    return internals;
  }

  it("asks the app instead of opening a window", async () => {
    const internals = await desktop(() => "id-token");
    const open = vi.fn();
    vi.stubGlobal("open", open);
    await expect(startReauth(config)()).resolves.toBe("id-token");
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_reauthenticate", undefined);
    expect(open).not.toHaveBeenCalled();
  });

  it("needs no re-authentication when the app runs in dev mode", async () => {
    await desktop(() => {
      throw { code: "wrong_mode", message: "dev" };
    });
    await expect(startReauth(config)()).resolves.toBeNull();
  });

  it.each([
    [{ code: "timeout", message: "t" }, "timeout"],
    [{ code: "cancelled", message: "c" }, "aborted"],
    [{ code: "rejected", message: "r" }, "denied"],
    // 古い版のアプリには、コマンドがない（型の付かない文字列のエラーになる）。
    ["Command zoo_reauthenticate not found", "desktopOutdated"],
  ])("maps %j to %s", async (failure, code) => {
    await desktop(() => {
      throw failure;
    });
    const result = startReauth(config)();
    await expect(result).rejects.toBeInstanceOf(ReauthError);
    await expect(result).rejects.toMatchObject({ code });
  });
});

describe("desktopSetBadge", () => {
  it("sends one call at a time and only the latest pending value", async () => {
    const releases: Array<() => void> = [];
    const internals = install((cmd) =>
      cmd === "zoo_session"
        ? session(DESKTOP_PROTOCOL_VERSION, "remote")
        : new Promise<void>((resolve) => {
            releases.push(resolve);
          }),
    );
    await detectDesktop();
    internals.invoke.mockClear();
    desktopSetBadge(5);
    desktopSetBadge(3);
    desktopSetBadge(0);
    expect(internals.invoke).toHaveBeenCalledTimes(1);
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_set_badge", { count: 5 });
    releases[0]?.();
    await vi.waitFor(() => expect(internals.invoke).toHaveBeenCalledTimes(2));
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_set_badge", { count: 0 });
    releases[1]?.();
  });

  it("clamps negative and non-finite counts to zero", async () => {
    const internals = install((cmd) =>
      cmd === "zoo_session" ? session(DESKTOP_PROTOCOL_VERSION, "remote") : undefined,
    );
    await detectDesktop();
    internals.invoke.mockClear();
    desktopSetBadge(Number.NaN);
    await vi.waitFor(() => expect(internals.invoke).toHaveBeenCalledTimes(1));
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_set_badge", { count: 0 });
    desktopSetBadge(-4);
    await vi.waitFor(() => expect(internals.invoke).toHaveBeenCalledTimes(2));
    expect(internals.invoke).toHaveBeenLastCalledWith("zoo_set_badge", { count: 0 });
  });
});
