// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { Composer } from "./Composer";

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
const send = vi.fn(async () => true);
beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  }));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  send.mockClear();
  await act(async () =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(Composer, {
          onSend: send,
          mentionUsers: [
            {
              id: "alice",
              user_name: "alice",
              display_name: "Alice",
              created_at: "2026-10-24T00:00:00Z",
              updated_at: null,
            },
          ],
          mentionMemberIds: new Set(["alice"]),
        }),
      }),
    ),
  );
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function input(value: string) {
  const textarea = container.querySelector("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(
      textarea,
      value,
    );
    textarea.setSelectionRange(value.length, value.length);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return textarea;
}
it("inserts a member mention with Enter without sending, then sends the inserted username", async () => {
  const textarea = await input("hello @ali");
  expect(container.querySelector('[role="listbox"]')?.textContent).toContain("@alice");
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  expect(textarea.value).toBe("hello @alice ");
  expect(send).not.toHaveBeenCalled();
  await act(async () => {
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
  expect(send).toHaveBeenCalledWith("hello @alice");
});
it("ignores IME Enter and dismisses suggestions with Escape", async () => {
  const textarea = await input("@ali");
  await act(async () => {
    textarea.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }),
    );
  });
  expect(textarea.value).toBe("@ali");
  expect(send).not.toHaveBeenCalled();
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
  expect(container.querySelector('[role="listbox"]')).toBeNull();
});
