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
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 0;
  });
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
function options() {
  return Array.from(container.querySelectorAll<HTMLElement>('[role="option"]'));
}
async function press(key: string) {
  const textarea = container.querySelector("textarea")!;
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
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
it("exposes suggestions as a listbox without nested interactive controls", async () => {
  const textarea = await input("@al");
  const listbox = container.querySelector('[role="listbox"]')!;
  const option = options()[0]!;
  expect(textarea.getAttribute("role")).toBeNull();
  expect(textarea.getAttribute("aria-expanded")).toBeNull();
  expect(textarea.getAttribute("aria-autocomplete")).toBe("list");
  expect(textarea.getAttribute("aria-controls")).toBe(listbox.id);
  expect(textarea.getAttribute("aria-activedescendant")).toBe(option.id);
  expect(option.tagName).toBe("LI");
  expect(option.getAttribute("tabindex")).toBe("-1");
  expect(option.getAttribute("aria-selected")).toBe("true");
  expect(option.querySelector("button")).toBeNull();
});
it("moves the active mention with arrow keys and inserts it with Enter", async () => {
  const textarea = await input("hi @");
  const [channel, here, alice] = options();
  expect(textarea.getAttribute("aria-activedescendant")).toBe(channel!.id);
  await press("ArrowDown");
  expect(textarea.getAttribute("aria-activedescendant")).toBe(here!.id);
  expect(here!.getAttribute("aria-selected")).toBe("true");
  await press("ArrowDown");
  expect(textarea.getAttribute("aria-activedescendant")).toBe(alice!.id);
  await press("ArrowUp");
  expect(textarea.getAttribute("aria-activedescendant")).toBe(here!.id);
  await press("Enter");
  expect(textarea.value).toBe("hi @here ");
  expect(send).not.toHaveBeenCalled();
});
it("selects a mention by clicking an option and keeps focus in the textarea", async () => {
  const textarea = await input("hi @al");
  textarea.focus();
  const option = options()[0]!;
  await act(async () => {
    option.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    option.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  expect(textarea.value).toBe("hi @alice ");
  expect(document.activeElement).toBe(textarea);
  expect(container.querySelector('[role="listbox"]')).toBeNull();

});

it("offers MFM after $ and inserts inside brackets with Tab without sending", async () => {
  const textarea = await input("hello $je");
  expect(container.querySelector('[role="listbox"]')?.textContent).toContain("$[jelly]");
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
  });
  expect(textarea.value).toBe("hello $[jelly ]");
  expect(send).not.toHaveBeenCalled();
  await act(async () => {
    await new Promise((resolve) => requestAnimationFrame(resolve));
  });
  expect(textarea.selectionStart).toBe("hello $[jelly ".length);
});

it("supports MFM arrow selection, Escape and IME without interfering with mentions", async () => {
  const textarea = await input("$");
  expect(container.querySelectorAll('[role="option"]').length).toBeGreaterThan(8);
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    textarea.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }),
    );
  });
  expect(textarea.value).toBe("$");
  expect(send).not.toHaveBeenCalled();
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
  expect(container.querySelector('[role="listbox"]')).toBeNull();
  await input("@ali");
  expect(container.querySelector('[role="listbox"]')?.textContent).toContain("@alice");
});

it("keeps suggestions visible during composition and leaves IME keys untouched", async () => {
  const textarea = container.querySelector("textarea")!;
  for (const value of ["＄ｊｅ", "＠ａｌ"]) {
    await act(async () =>
      textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })),
    );
    await input(value);
    expect(container.querySelector('[role="listbox"]')?.textContent).toContain(
      value.startsWith("＄") ? "$[jelly]" : "@alice",
    );
    for (const key of ["Enter", "ArrowDown", "Tab", "Escape"]) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      await act(async () => textarea.dispatchEvent(event));
      expect(event.defaultPrevented).toBe(false);
      expect(textarea.value).toBe(value);
      expect(container.querySelector('[role="listbox"]')).not.toBeNull();
    }
    expect(send).not.toHaveBeenCalled();
    await act(async () =>
      textarea.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })),
    );
    const imeEnter = new KeyboardEvent("keydown", {
      key: "Enter",
      keyCode: 229,
      bubbles: true,
      cancelable: true,
    });
    await act(async () => textarea.dispatchEvent(imeEnter));
    expect(textarea.value).toBe(value);
    await act(async () =>
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })),
    );
    expect(textarea.value).toBe(value.startsWith("＄") ? "$[jelly ]" : "@alice ");
  }
});
