import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { AuthGate } from "./AuthGate";
import { notifyUnauthorized } from "../../lib/auth";
import { resetDesktopForTest } from "../../lib/desktop";
import { markLoggedOut } from "../../lib/oidc/pending";

const mocks = vi.hoisted(() => ({
  client: {
    hasSession: true,
    isCallbackUrl: () => false,
    getAccessToken: vi.fn<() => Promise<string | null>>(),
    setReauthorizeHandler: vi.fn(),
    resetSession: vi.fn(),
    login: vi.fn(async () => "https://id.example/authorize"),
  },
  router: { history: { replace: vi.fn() }, load: vi.fn(async () => {}) },
}));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => mocks.router }));
vi.mock("../../lib/oidc/session", () => ({ getOidcClient: () => mocks.client }));
vi.mock("../../lib/config", async (original) => ({
  ...(await original<typeof import("../../lib/config")>()),
  resolveConfig: () => ({ oidc: { issuer: "https://id.example", clientId: "zoo" } }),
}));

beforeEach(() => {
  sessionStorage.clear();
  mocks.client.login.mockClear();
  mocks.client.getAccessToken.mockReset();
});
afterEach(() => {
  sessionStorage.clear();
});

it("does not restart login when an in-flight request returns 401 after logout", async () => {
  render(
    <LocaleProvider>
      <AuthGate>
        <button>Cached history</button>
      </AuthGate>
    </LocaleProvider>,
  );
  await screen.findByRole("button", { name: "Cached history" });
  await act(async () => {
    markLoggedOut(sessionStorage);
    notifyUnauthorized();
  });
  await screen.findByRole("heading", { name: "ログアウトしました" });
  expect(mocks.client.login).not.toHaveBeenCalled();
  expect(sessionStorage.getItem("zoo.oidc.loggedOut")).toBe("1");
});

it.each(["token", null])("rechecks a restored session on reconnect (token: %s)", async (token) => {
  mocks.client.getAccessToken.mockResolvedValue(token);
  render(
    <LocaleProvider>
      <AuthGate>
        <button>Cached history</button>
      </AuthGate>
    </LocaleProvider>,
  );
  await screen.findByRole("button", { name: "Cached history" });
  await act(async () => {
    window.dispatchEvent(new Event("online"));
  });
  await waitFor(() => expect(mocks.client.getAccessToken).toHaveBeenCalledOnce());
  if (token) {
    expect(mocks.client.login).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Cached history" })).toBeTruthy();
  } else {
    await waitFor(() => expect(mocks.client.login).toHaveBeenCalledOnce());
  }
});

it.each([
  ["a protocol mismatch", async () => ({ protocol: 999, ui: "remote" })],
  [
    "a failed zoo_session",
    async () => {
      throw new Error("ipc failed");
    },
  ],
])("shows an update notice instead of OIDC login in Tauri on %s", async (_, invoke) => {
  const win = window as unknown as { __TAURI_INTERNALS__?: unknown };
  win.__TAURI_INTERNALS__ = { invoke: vi.fn(invoke), transformCallback: vi.fn() };
  resetDesktopForTest();
  mocks.client.hasSession = false;
  try {
    render(
      <LocaleProvider>
        <AuthGate>
          <button>Cached history</button>
        </AuthGate>
      </LocaleProvider>,
    );
    await screen.findByRole("heading", { name: "デスクトップ版の更新が必要です" });
    expect(screen.getByRole("button", { name: "再読み込み" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Cached history" })).toBeNull();
    expect(mocks.client.login).not.toHaveBeenCalled();
  } finally {
    delete win.__TAURI_INTERNALS__;
    resetDesktopForTest();
    mocks.client.hasSession = true;
  }
});
