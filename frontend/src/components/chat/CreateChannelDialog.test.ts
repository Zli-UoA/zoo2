// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError } from "../../lib/api/client";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { CreateChannelDialog } from "./CreateChannelDialog";

const { insert } = vi.hoisted(() => ({ insert: vi.fn() }));
vi.mock("../../lib/electric/collections", () => ({
  getCollections: () => ({ channels: { insert } }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "user-1", isLoading: false }),
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
function nameInput() {
  return document.querySelector('input[aria-label="新しいチャンネル名"]') as HTMLInputElement;
}
function descriptionInput() {
  return document.querySelector("textarea") as HTMLTextAreaElement;
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
async function render(open = true) {
  await flush(() =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(CreateChannelDialog, { open, onOpenChange }),
      }),
    ),
  );
}
beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  localStorage.setItem("zoo-locale", "ja");
  insert.mockReset();
  onOpenChange.mockReset();
  insert.mockReturnValue({ when: () => Promise.resolve() });
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
describe("CreateChannelDialog", () => {
  it("starts with empty fields and disables submit without a name", () => {
    expect(nameInput().value).toBe("");
    expect(descriptionInput().value).toBe("");
    expect(submit().disabled).toBe(true);
  });

  it("creates a channel with trimmed name and description", async () => {
    await setValue(nameInput(), "  new-channel  ");
    await setValue(descriptionInput(), "  新しいチャンネルの説明  ");
    await send();
    expect(insert).toHaveBeenCalledWith(
      expect.objectContaining({
        channel_name: "new-channel",
        description: "新しいチャンネルの説明",
        created_user_id: "user-1",
        is_private: false,
        updated_at: null,
        archive_at: null,
        deleted_at: null,
      }),
    );
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("sends null when the description is empty or whitespace", async () => {
    await setValue(nameInput(), "new-channel");
    await setValue(descriptionInput(), "   ");
    await send();
    expect(insert).toHaveBeenCalledWith(expect.objectContaining({ description: null }));
  });

  it("shows an error and blocks submit when the description exceeds 250 characters", async () => {
    await setValue(nameInput(), "new-channel");
    await setValue(descriptionInput(), "あ".repeat(251));
    expect(submit().disabled).toBe(true);
    expect(document.querySelector('[id="create-channel-description-help"]')?.textContent).toContain(
      "250文字以内",
    );
    expect(descriptionInput().getAttribute("aria-invalid")).toBe("true");
    await send();
    expect(insert).not.toHaveBeenCalled();
  });

  it("keeps the input and shows a localized error when creation fails", async () => {
    insert.mockImplementation(() => {
      throw new ApiError(403, "Forbidden", {}, { method: "POST", path: "/api/channels" });
    });
    await setValue(nameInput(), "new-channel");
    await setValue(descriptionInput(), "説明");
    await send();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "この操作を行う権限がありません",
    );
    expect(nameInput().value).toBe("new-channel");
    expect(descriptionInput().value).toBe("説明");
    expect(submit().disabled).toBe(false);
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("resets the form when cancelled and when it reopens", async () => {
    await setValue(nameInput(), "discard");
    await setValue(descriptionInput(), "破棄する説明");
    const cancel = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "キャンセル",
    )!;
    await flush(() => cancel.click());
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
    await render(false);
    await render(true);
    expect(nameInput().value).toBe("");
    expect(descriptionInput().value).toBe("");
    expect(submit().disabled).toBe(true);
  });
});
