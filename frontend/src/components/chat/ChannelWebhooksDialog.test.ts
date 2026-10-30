// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelWebhooksDialog, webhookUrl } from "./ChannelWebhooksDialog";
const { api, copy } = vi.hoisted(() => ({
  api: { requestWithTx: vi.fn(), post: vi.fn(), delete: vi.fn() },
  copy: vi.fn(),
}));
vi.mock("../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../lib/api/client")>()),
  createConfiguredApiClient: () => api,
}));
vi.mock("../../lib/electric/collections", () => ({
  getCollections: () => ({ config: { apiBaseUrl: "https://zoo.example" } }),
}));
vi.mock("../../hooks/useChannelAdmin", () => ({ describeApiError: () => "読み込みエラー" }));
const hook = {
  id: "hook-1",
  name: "Webhook",
  path: "/api/webhooks/hook-1?token=url-token",
  created_at: "2026-10-23",
};
const channel = {
  id: "channel-1",
  channel_name: "general",
  description: null,
  created_user_id: "owner",
  is_private: false,
  is_dm: false,
  created_at: "2026-10-23",
  updated_at: null,
  archive_at: null,
  deleted_at: null,
};
let root: Root;
let container: HTMLDivElement;
async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}
function button(text: string) {
  return Array.from(document.querySelectorAll("button")).find((b) => b.textContent === text)!;
}
async function mount(archived = false) {
  await flush(() =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(ChannelWebhooksDialog, {
          channel: { ...channel, archive_at: archived ? "2026-10-23" : null },
          onOpenChange: vi.fn(),
        }),
      }),
    ),
  );
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.setItem("zoo-locale", "ja");
  vi.clearAllMocks();
  api.requestWithTx.mockResolvedValue({ data: [hook], txid: null });
  copy.mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: copy } });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await flush(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
describe("channel webhook management", () => {
  it("builds URLs for same-origin and separate API hosts", () => {
    expect(webhookUrl("", hook.path, "https://zoo.example")).toBe(
      "https://zoo.example" + hook.path,
    );
    expect(webhookUrl("https://api.example/", hook.path, "https://zoo.example")).toBe(
      "https://api.example" + hook.path,
    );
  });
  it("shows one masked webhook URL and copies the complete tokenized URL", async () => {
    await mount();
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(1);
    expect(document.querySelector('[aria-label="Slack Webhook URLをコピー"]')).toBeNull();
    expect(document.querySelector('[aria-label="Secretをコピー"]')).toBeNull();
    await flush(() =>
      (document.querySelector('[aria-label="Webhook URLをコピー"]') as HTMLButtonElement).click(),
    );
    expect(copy).toHaveBeenCalledWith("https://zoo.example" + hook.path);
    await flush(() =>
      (document.querySelector('[aria-label="Webhook URLを表示"]') as HTMLButtonElement).click(),
    );
    expect(document.querySelector('input[type="password"]')).toBeNull();
    expect(document.querySelector("input[readonly]")?.getAttribute("value")).toBe(
      "https://zoo.example" + hook.path,
    );
  });
  it("creates, rotates and deletes with confirmation", async () => {
    await mount();
    api.post.mockResolvedValueOnce({ data: { ...hook, id: "hook-2" } });
    await flush(() =>
      document
        .querySelector("form")!
        .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
    );
    expect(api.post).toHaveBeenCalledWith("/api/channels/channel-1/webhooks", { name: "Webhook" });
    api.post.mockResolvedValueOnce({
      data: { ...hook, path: "/api/webhooks/hook-1?token=new-token" },
    });
    await flush(() => button("Webhook URLを再発行").click());
    expect(api.post).toHaveBeenCalledTimes(1);
    await flush(() => button("実行").click());
    expect(api.post).toHaveBeenLastCalledWith(
      "/api/channels/channel-1/webhooks/hook-1/rotate-secret",
    );
    expect((document.querySelector('input[type="password"]') as HTMLInputElement).value).toBe(
      "https://zoo.example/api/webhooks/hook-1?token=new-token",
    );
    api.delete.mockResolvedValue({ data: hook.id });
    await flush(() => button("削除").click());
    expect(api.delete).not.toHaveBeenCalled();
    await flush(() => button("実行").click());
    expect(api.delete).toHaveBeenCalledWith("/api/channels/channel-1/webhooks/hook-1");
    expect(document.querySelectorAll('input[type="password"]')).toHaveLength(1);
  });
  it("disables creation in archived channels", async () => {
    await mount(true);
    expect(button("作成").disabled).toBe(true);
  });
  it("reports loading failures and supports retry", async () => {
    api.requestWithTx.mockRejectedValueOnce(new Error("failed"));
    await mount();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("読み込みエラー");
    await flush(() => button("再読み込み").click());
    expect(api.requestWithTx).toHaveBeenCalledTimes(2);
    expect(document.querySelector('input[type="password"]')).not.toBeNull();
  });
});
