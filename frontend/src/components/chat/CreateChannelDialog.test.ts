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
// fine pointer なし（モバイル等）を明示する。
vi.mock("../layout/useMediaQuery", () => ({ useMediaQuery: () => false }));

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
function nameHelp() {
  return document.querySelector('[id="create-channel-name-help"]') as HTMLParagraphElement;
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
  it("starts with empty fields and keeps submit clickable to show the required error", () => {
    expect(nameInput().value).toBe("");
    expect(descriptionInput().value).toBe("");
    // 空欄でも押せるようにして、エラー表示とフォーカス移動に到達できるようにする。
    expect(submit().disabled).toBe(false);
  });

  it("exposes form attributes and does not autofocus without a fine pointer", () => {
    expect(nameInput().getAttribute("name")).toBe("channel_name");
    // HTML の maxlength は UTF-16 単位で切り詰めるため付けない。上限は tooLong エラーで伝える。
    expect(nameInput().hasAttribute("maxlength")).toBe(false);
    expect(nameInput().getAttribute("autocomplete")).toBe("off");
    expect(nameInput().getAttribute("aria-describedby")).toBe("create-channel-name-help");
    // React 19 は autoFocus を属性として残さず focus() を呼ぶため、実フォーカスで確認する。
    expect(document.activeElement).not.toBe(nameInput());
    expect(nameHelp().textContent).toContain("80文字以内");
  });

  it("validates an unsupported name inline, blocks submit, and focuses the error", async () => {
    await setValue(nameInput(), "Invalid Name");
    expect(nameInput().getAttribute("aria-invalid")).toBe("true");
    expect(nameHelp().textContent).toContain("半角ハイフン");
    // 押したときにエラー欄へ案内できるよう、ボタン自体は有効なままにする。
    expect(submit().disabled).toBe(false);
    await send();
    expect(insert).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(nameHelp());
  });

  it("explains reserved names without rejecting valid mixed-case names", async () => {
    await setValue(nameInput(), "general");
    expect(nameHelp().textContent).toContain("予約語");
    expect(nameHelp().textContent).toContain("general");
    await setValue(nameInput(), "General");
    expect(nameInput().getAttribute("aria-invalid")).toBe("false");
    await send();
    expect(insert).toHaveBeenCalledWith(expect.objectContaining({ channel_name: "General" }));
  });

  it("accepts non-ASCII letters in a channel name", async () => {
    await setValue(nameInput(), "équipe");
    await send();
    expect(insert).toHaveBeenCalledWith(expect.objectContaining({ channel_name: "équipe" }));
  });

  it("reports an overlong name without truncating the input", async () => {
    const name = "あ".repeat(81);
    await setValue(nameInput(), name);
    expect(nameInput().value).toBe(name);
    expect(nameInput().getAttribute("aria-invalid")).toBe("true");
    expect(nameHelp().textContent).toContain("80文字以内");
    await send();
    expect(insert).not.toHaveBeenCalled();
  });

  it("accepts a name with surrogate pairs within the code point limit", async () => {
    // 𠮷 は 1 コードポイント（UTF-16 では 2 コード単位）。バックエンドと同じくコードポイントで数える。
    const name = "𠮷".repeat(80);
    await setValue(nameInput(), name);
    expect(nameInput().value).toBe(name);
    expect(nameInput().getAttribute("aria-invalid")).toBe("false");
    await send();
    expect(insert).toHaveBeenCalledWith(expect.objectContaining({ channel_name: name }));
  });

  it("reports a missing name and focuses the help text when the button is clicked", async () => {
    await flush(() => submit().click());
    expect(nameInput().getAttribute("aria-invalid")).toBe("true");
    expect(nameHelp().textContent).toContain("入力してください");
    expect(document.activeElement).toBe(nameHelp());
    expect(insert).not.toHaveBeenCalled();
  });

  it("clears the inline error once the name becomes valid", async () => {
    await setValue(nameInput(), "Invalid Name");
    expect(nameInput().getAttribute("aria-invalid")).toBe("true");
    await setValue(nameInput(), "valid-name");
    expect(nameInput().getAttribute("aria-invalid")).toBe("false");
    expect(nameHelp().textContent).toContain("80文字以内");
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
    expect(submit().disabled).toBe(false);
  });
});
