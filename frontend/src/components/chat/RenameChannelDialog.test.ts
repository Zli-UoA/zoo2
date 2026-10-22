// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError } from "../../lib/api/client";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { RenameChannelDialog } from "./RenameChannelDialog";

const { update } = vi.hoisted(() => ({ update: vi.fn() }));
vi.mock("../../lib/electric/collections", () => ({
  getCollections: () => ({ channels: { update } }),
}));

let root: Root;
let container: HTMLDivElement;
const onOpenChange = vi.fn();
async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}
function input() {
  return document.querySelector("input")!;
}
function submit() {
  return document.querySelector('button[type="submit"]') as HTMLButtonElement;
}
async function rename(name: string) {
  await flush(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), name);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function send() {
  await flush(() =>
    document
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}
beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  localStorage.setItem("zoo-locale", "ja");
  update.mockReset();
  onOpenChange.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await flush(() =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(RenameChannelDialog, {
          channel: {
            id: "channel-1",
            channel_name: "original",
            description: null,
            created_user_id: "user-1",
            is_private: false,
            created_at: "2026-01-01T00:00:00Z",
            updated_at: null,
            archive_at: null,
            deleted_at: null,
          },
          onOpenChange,
        }),
      }),
    ),
  );
});
afterEach(async () => {
  await flush(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
describe("RenameChannelDialog", () => {
  it("starts with the current name and disables unchanged or empty names", async () => {
    expect(input().value).toBe("original");
    expect(submit().disabled).toBe(true);
    await rename("");
    expect(submit().disabled).toBe(true);
    await send();
    expect(update).not.toHaveBeenCalled();
  });
  it("updates only the name and waits for persistence before closing", async () => {
    let settle!: () => void;
    const settled = new Promise<void>((resolve) => {
      settle = resolve;
    });
    const draft = { channel_name: "original", description: "keep" };
    const when = vi.fn(() => settled);
    update.mockImplementation((_id, change) => {
      change(draft);
      return { when };
    });
    await rename("renamed");
    await send();
    expect(update).toHaveBeenCalledWith("channel-1", expect.any(Function));
    expect(draft).toEqual({ channel_name: "renamed", description: "keep" });
    expect(when).toHaveBeenCalledWith("settled");
    expect(submit().disabled).toBe(true);
    expect(onOpenChange).not.toHaveBeenCalled();
    await send();
    expect(update).toHaveBeenCalledTimes(1);
    await flush(settle);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
  it.each(["開発チーム-ざつだん-123", "𠮷".repeat(80), "Team-開発-相談"])(
    "accepts and persists channel name %s",
    async (name) => {
      const draft = { channel_name: "original" };
      update.mockImplementation((_id, change) => {
        change(draft);
        return { when: () => Promise.resolve() };
      });
      await rename(name);
      expect(input().checkValidity()).toBe(true);
      await send();
      expect(draft.channel_name).toBe(name);
      expect(onOpenChange).toHaveBeenCalledWith(false);
    },
  );
  it("keeps the edited name and shows a duplicate error so the user can retry", async () => {
    update.mockImplementation(() => {
      throw new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/api/channels/channel-1" });
    });
    await rename("duplicate");
    await send();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("同名のチャンネル");
    expect(input().value).toBe("duplicate");
    expect(submit().disabled).toBe(false);
    expect(onOpenChange).not.toHaveBeenCalled();
  });
  it("cancels without writing", async () => {
    await rename("discard");
    const cancel = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "キャンセル",
    )!;
    await flush(() => cancel.click());
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(update).not.toHaveBeenCalled();
  });
});
