// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError } from "../../lib/api/client";
import type { ChannelRow } from "../../lib/electric/types";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { RenameChannelDialog } from "./RenameChannelDialog";

const { update } = vi.hoisted(() => ({ update: vi.fn() }));
vi.mock("../../lib/electric/collections", () => ({
  getCollections: () => ({ channels: { update } }),
}));

const channel: ChannelRow = {
  id: "channel-1",
  channel_name: "original",
  description: "keep",
  created_user_id: "user-1",
  is_private: false,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: null,
  archive_at: null,
  deleted_at: null,
};

let root: Root;
let container: HTMLDivElement;
const onOpenChange = vi.fn();
async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}
function nameInput() {
  return document.querySelector("input")!;
}
function descriptionInput() {
  return document.querySelector("textarea")!;
}
function submit() {
  return document.querySelector('button[type="submit"]') as HTMLButtonElement;
}
async function setValue(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const prototype =
    element instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  await flush(() => {
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function send() {
  await flush(() =>
    document
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}
async function render(channelOverrides: Partial<ChannelRow> = {}) {
  await flush(() =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(RenameChannelDialog, {
          channel: { ...channel, ...channelOverrides },
          onOpenChange,
        }),
      }),
    ),
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
  await render();
});
afterEach(async () => {
  await flush(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
describe("RenameChannelDialog", () => {
  it("starts with the current name and description and disables unchanged values", async () => {
    expect(nameInput().value).toBe("original");
    expect(descriptionInput().value).toBe("keep");
    expect(submit().disabled).toBe(true);
  });

  it("disables submit for an empty name", async () => {
    await setValue(nameInput(), "");
    expect(submit().disabled).toBe(true);
    await send();
    expect(update).not.toHaveBeenCalled();
  });

  it("updates only the name and waits for persistence before closing", async () => {
    let settle!: () => void;
    const settled = new Promise<void>((resolve) => {
      settle = resolve;
    });
    const draft: Partial<ChannelRow> = { channel_name: "original", description: "keep" };
    const when = vi.fn(() => settled);
    update.mockImplementation((_id, change) => {
      change(draft);
      return { when };
    });
    await setValue(nameInput(), "renamed");
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

  it("updates only the description with trimmed text", async () => {
    const draft: Partial<ChannelRow> = { channel_name: "original", description: "keep" };
    update.mockImplementation((_id, change) => {
      change(draft);
      return { when: () => Promise.resolve() };
    });
    await setValue(descriptionInput(), "  新しい説明  ");
    expect(submit().disabled).toBe(false);
    await send();
    expect(draft).toEqual({ channel_name: "original", description: "新しい説明" });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("does not send a stale description after an external update when only the name is edited", async () => {
    const draft: Partial<ChannelRow> = { channel_name: "original", description: "external" };
    update.mockImplementation((_id, change) => {
      change(draft);
      return { when: () => Promise.resolve() };
    });
    await render({ description: "external" });
    await setValue(nameInput(), "renamed");
    await send();
    expect(draft).toEqual({ channel_name: "renamed", description: "external" });
  });

  it("does not send a stale name after an external update when only the description is edited", async () => {
    const draft: Partial<ChannelRow> = { channel_name: "external-name", description: "keep" };
    update.mockImplementation((_id, change) => {
      change(draft);
      return { when: () => Promise.resolve() };
    });
    await render({ channel_name: "external-name" });
    await setValue(descriptionInput(), "新しい説明");
    await send();
    expect(draft).toEqual({ channel_name: "external-name", description: "新しい説明" });
  });

  it("clears the description by saving an empty string", async () => {
    const draft: Partial<ChannelRow> = { channel_name: "original", description: "keep" };
    update.mockImplementation((_id, change) => {
      change(draft);
      return { when: () => Promise.resolve() };
    });
    await setValue(descriptionInput(), "   ");
    await send();
    expect(draft.description).toBe("");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("shows an error and blocks submit when the description exceeds 250 characters", async () => {
    await setValue(descriptionInput(), "あ".repeat(251));
    expect(submit().disabled).toBe(true);
    expect(document.querySelector('[id="rename-channel-description-help"]')?.textContent).toContain(
      "250文字以内",
    );
    expect(descriptionInput().getAttribute("aria-invalid")).toBe("true");
    await send();
    expect(update).not.toHaveBeenCalled();
  });

  it.each(["開発チーム_ざつだん-123", "𠮷".repeat(80), "Team-開発-相談"])(
    "accepts and persists channel name %s",
    async (name) => {
      const draft: Partial<ChannelRow> = { channel_name: "original" };
      update.mockImplementation((_id, change) => {
        change(draft);
        return { when: () => Promise.resolve() };
      });
      await setValue(nameInput(), name);
      expect(nameInput().checkValidity()).toBe(true);
      await send();
      expect(draft.channel_name).toBe(name);
      expect(onOpenChange).toHaveBeenCalledWith(false);
    },
  );

  it("keeps the edited values and shows a duplicate error so the user can retry", async () => {
    update.mockImplementation(() => {
      throw new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/api/channels/channel-1" });
    });
    await setValue(nameInput(), "duplicate");
    await setValue(descriptionInput(), "説明も残す");
    await send();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("同名のチャンネル");
    expect(nameInput().value).toBe("duplicate");
    expect(descriptionInput().value).toBe("説明も残す");
    expect(submit().disabled).toBe(false);
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("cancels without writing", async () => {
    await setValue(nameInput(), "discard");
    await setValue(descriptionInput(), "discard description");
    const cancel = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "キャンセル",
    )!;
    await flush(() => cancel.click());
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(update).not.toHaveBeenCalled();
  });
});
