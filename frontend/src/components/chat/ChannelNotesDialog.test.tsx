// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelNotesTabs, moveNoteToDropPosition } from "./ChannelNotesTabs";
import { ChannelScreen } from "./ChannelScreen";
import { ApiError } from "../../lib/api/client";

const { api, config, onNoteChange, onMessageTab } = vi.hoisted(() => ({
  api: { requestWithTx: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn() },
  config: { apiBaseUrl: "https://zoo.example" },
  onNoteChange: vi.fn(),
  onMessageTab: vi.fn(),
}));
vi.mock("../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../lib/api/client")>()),
  createConfiguredApiClient: () => api,
}));
vi.mock("../../lib/electric/collections-context", () => ({ useCollections: () => ({ config }) }));
vi.mock("@tanstack/react-router", () => ({ useBlocker: vi.fn() }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: [] }),
  useChannelMembers: () => ({ data: [] }),
  useActiveChannels: () => ({ data: [{ id: "channel-1" }] }),
  useChannelAccess: () => ({ canRead: true, isMember: true, isArchived: false, members: [] }),
  useMessageFeed: () => ({ data: [] }),
}));
vi.mock("../layout/shell-context", () => ({ useShell: () => ({ showSidebar: vi.fn() }) }));
vi.mock("./ChannelHeader", () => ({ ChannelHeader: () => null }));
vi.mock("./messages/MessagePane", () => ({
  MessagePane: () => createElement("textarea", { "aria-label": "draft" }),
}));
vi.mock("./messages/MfmContent", () => ({
  MfmContent: ({ content }: { content: string }) => createElement("div", null, content),
}));

const note = {
  id: "note-1",
  title: "Initial",
  content: "$[x2 MFM]",
  version: 1,
  updated_at: "2026-11-04",
};
let root: Root;
let container: HTMLDivElement;
async function flush(action: () => unknown) {
  await act(async () => {
    action();
    for (let tick = 0; tick < 6; tick += 1) await Promise.resolve();
  });
}
function button(label: string) {
  return Array.from(document.body.querySelectorAll("button")).find((item) =>
    item.textContent?.includes(label),
  )!;
}
async function mount(noteId?: string, options: { canWrite?: boolean; archived?: boolean } = {}) {
  await flush(() =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(ChannelNotesTabs, {
          channelId: "channel-1",
          canWrite: options.canWrite ?? true,
          archived: options.archived ?? false,
          noteId,
          onNoteChange,
          onMessageTab,
        }),
      }),
    ),
  );
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  if (!document.doctype)
    document.insertBefore(
      document.implementation.createDocumentType("html", "", ""),
      document.documentElement,
    );
  localStorage.setItem("zoo-locale", "ja");
  vi.clearAllMocks();
  api.patch.mockReset();
  api.requestWithTx.mockResolvedValue({ data: [note], txid: null });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await flush(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("ChannelNotesTabs", () => {
  it("keeps the message composer mounted when switching to a note", async () => {
    const renderScreen = (noteId?: string) =>
      root.render(
        createElement(LocaleProvider, {
          children: createElement(ChannelScreen, { channelId: "channel-1", noteId }),
        }),
      );
    await flush(() => renderScreen());
    const draft = document.querySelector('textarea[aria-label="draft"]') as HTMLTextAreaElement;
    draft.value = "unsent message";
    await flush(() => renderScreen("note-1"));
    await flush(() => renderScreen());
    expect(document.querySelector('textarea[aria-label="draft"]')).toBe(draft);
    expect(draft.value).toBe("unsent message");
  });
  it("does not cancel a slow initial load when polling starts", async () => {
    vi.useFakeTimers();
    try {
      let resolveLoad!: (value: { data: (typeof note)[]; txid: null }) => void;
      api.requestWithTx.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveLoad = resolve;
          }),
      );
      await mount("note-1");
      const signal = api.requestWithTx.mock.calls[0]?.[1]?.signal as AbortSignal;
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
      expect(signal.aborted).toBe(false);
      await flush(() => resolveLoad({ data: [note], txid: null }));
      expect(document.body.textContent).toContain("Initial");
    } finally {
      vi.useRealTimers();
    }
  });
  it("computes exact before and after insertion order for drag targets", () => {
    expect(moveNoteToDropPosition(["a", "b", "c"], 0, 2, false)).toEqual(["b", "a", "c"]);
    expect(moveNoteToDropPosition(["a", "b", "c"], 0, 2, true)).toEqual(["b", "c", "a"]);
    expect(moveNoteToDropPosition(["a", "b", "c"], 2, 0, false)).toEqual(["c", "a", "b"]);
    expect(moveNoteToDropPosition(["a", "b", "c"], 2, 0, true)).toEqual(["a", "c", "b"]);
  });
  it("shows a target insertion line for both halves and clears it on drag end", async () => {
    const notes = ["a", "b", "c"].map((id) => ({ ...note, id, title: id.toUpperCase() }));
    api.requestWithTx.mockResolvedValue({ data: notes, txid: null });
    await mount();
    const target = Array.from(document.body.querySelectorAll('[role="tab"]')).find(
      (item) => item.textContent === "B",
    )!.parentElement!;
    const source = Array.from(document.body.querySelectorAll('[role="tab"]')).find(
      (item) => item.textContent === "A",
    )!.parentElement!;
    await flush(() => source.dispatchEvent(new Event("dragstart", { bubbles: true })));
    vi.spyOn(target, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      left: 0,
      right: 100,
      top: 0,
      bottom: 40,
      width: 100,
      height: 40,
      toJSON: () => ({}),
    });
    await flush(() =>
      target.dispatchEvent(new Event("dragover", { bubbles: true, cancelable: true })),
    );
    expect(target.className).toContain("border-l-2");
    const dragEvent = new Event("dragover", { bubbles: true, cancelable: true });
    Object.defineProperty(dragEvent, "clientX", { value: 90 });
    await flush(() => target.dispatchEvent(dragEvent));
    expect(target.className).toContain("border-r-2");
    await flush(() => target.dispatchEvent(new Event("dragend", { bubbles: true })));
    expect(target.className).not.toContain("border-r-2");
  });
  it("keeps reorder buttons visible on mobile and available on keyboard focus on desktop", async () => {
    await mount();
    const move = document.body.querySelector(
      '[aria-label="下へ移動 Initial"]',
    ) as HTMLButtonElement;
    expect(move.parentElement?.className).toContain("md:sr-only");
    expect(move.parentElement?.className).toContain("md:focus-within:not-sr-only");
    expect(move.parentElement?.className).toContain("flex");
  });
  it("shows the message tab and channel-wide note tabs, then opens a note for its direct URL", async () => {
    await mount();
    expect(document.body.querySelector('[role="tablist"]')?.textContent).toContain("メッセージ");
    expect(document.body.querySelector('[role="tablist"]')?.textContent).toContain("Initial");
    await flush(() => button("Initial").click());
    expect(onNoteChange).toHaveBeenCalledWith("note-1");
  });

  it("loads a direct note URL and enters explicit edit mode while preserving raw MFM text", async () => {
    await mount("note-1");
    expect(document.body.textContent).toContain("Initial");
    expect(document.body.textContent).toContain("$[x2 MFM]");
    expect(document.body.textContent).not.toContain("保存済み");
    await flush(() => button("編集").click());
    expect(document.body.textContent).not.toContain("保存済み");
    expect(document.body.querySelector("textarea")).toBeTruthy();
    const title = document.body.querySelector("input") as HTMLInputElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(title), "value")!.set!.call(
        title,
        "Renamed",
      );
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.body.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe(
      "Renamed",
    );
  });

  it("loads the URL-selected note after navigation from an edited note", async () => {
    const second = { ...note, id: "note-2", title: "Second", content: "Second body", version: 3 };
    api.requestWithTx.mockResolvedValue({ data: [note, second], txid: null });
    await mount("note-1");
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "unsaved draft",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await mount("note-2");
    expect(document.body.textContent).toContain("Second body");
    expect(document.body.querySelector("textarea")).toBeNull();
    await flush(() => button("編集").click());
    const nextContent = document.body.querySelector("textarea")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(nextContent), "value")!.set!.call(
        nextContent,
        "changed second",
      );
      nextContent.dispatchEvent(new Event("input", { bubbles: true }));
    });
    api.patch.mockResolvedValueOnce({ data: { ...second, content: "changed second", version: 4 } });
    await flush(() => button("ノートを保存").click());
    expect(api.patch).toHaveBeenLastCalledWith("/api/channels/channel-1/notes/note-2", {
      title: "Second",
      content: "changed second",
      version: 3,
    });
  });

  it("ignores an old note save response after URL navigation", async () => {
    const second = { ...note, id: "note-2", title: "Second", content: "Second body", version: 3 };
    api.requestWithTx.mockResolvedValue({ data: [note, second], txid: null });
    let resolvePatch!: (value: unknown) => void;
    api.patch.mockReturnValueOnce(new Promise((resolve) => (resolvePatch = resolve)));
    await mount("note-1");
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "changed first",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flush(() => button("ノートを保存").click());
    await mount("note-2");
    expect(document.body.textContent).toContain("Second body");
    await flush(() => resolvePatch({ data: { ...note, content: "changed first", version: 2 } }));
    expect(document.body.textContent).toContain("Second body");
    expect(document.body.textContent).not.toContain("changed first");
  });

  it.each(["create", "delete"])(
    "ignores a pending %s response after navigation",
    async (action) => {
      const second = { ...note, id: "note-2", title: "Second", content: "Second body", version: 3 };
      api.requestWithTx.mockResolvedValue({ data: [note, second], txid: null });
      vi.stubGlobal(
        "confirm",
        vi.fn(() => true),
      );
      let finish!: (value: unknown) => void;
      const pending = new Promise((resolve) => (finish = resolve));
      if (action === "create") api.post.mockReturnValueOnce(pending);
      else api.delete.mockReturnValueOnce(pending);
      await mount("note-1");
      await flush(() => {
        if (action === "create")
          (document.body.querySelector('[aria-label="ノートを作成"]') as HTMLButtonElement).click();
        else
          Array.from(document.body.querySelectorAll("button"))
            .find((b) => b.textContent === "削除")!
            .click();
      });
      await mount("note-2");
      await flush(() => finish({ data: { ...note, id: "new-note" } }));
      expect(document.body.textContent).toContain("Second body");
      expect(onNoteChange).not.toHaveBeenCalled();
    },
  );

  it("ignores an old latest-version retry response after navigation", async () => {
    const second = { ...note, id: "note-2", title: "Second", content: "Second body", version: 3 };
    const original = { ...note, content: "- [ ] task" };
    let finish!: (value: unknown) => void;
    api.requestWithTx
      .mockResolvedValueOnce({ data: [original, second] })
      .mockRejectedValueOnce(new Error("offline"))
      .mockReturnValueOnce(new Promise((resolve) => (finish = resolve)))
      .mockResolvedValue({ data: [original, second] });
    api.patch.mockRejectedValueOnce(
      new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
    );
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await flush(() => button("再試行").click());
    await mount("note-2");
    await flush(() => finish({ data: { ...note, content: "old latest", version: 2 } }));
    await flush(() => button("編集").click());
    expect(document.body.textContent).not.toContain("old latest");
    expect(button("上書きする")).toBeUndefined();
  });

  it("ignores an old checkbox conflict after navigation", async () => {
    const second = { ...note, id: "note-2", title: "Second", content: "Second body", version: 3 };
    api.requestWithTx.mockResolvedValue({ data: [{ ...note, content: "- [ ] task" }, second] });
    let rejectPatch!: (reason: unknown) => void;
    api.patch.mockReturnValueOnce(new Promise((_, reject) => (rejectPatch = reject)));
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await mount("note-2");
    await flush(() =>
      rejectPatch(new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" })),
    );
    expect(document.body.querySelector('[role="alert"]')).toBeNull();
    expect(document.body.textContent).toContain("Second body");
  });

  it("saves note edits with Ctrl+Enter but leaves Enter and IME composition alone", async () => {
    api.patch.mockResolvedValue({ data: { ...note, content: "changed", version: 2 }, txid: "2" });
    await mount("note-1");
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "changed",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flush(() =>
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })),
    );
    expect(api.patch).not.toHaveBeenCalled();
    await flush(() =>
      textarea.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          ctrlKey: true,
          isComposing: true,
          bubbles: true,
        }),
      ),
    );
    expect(api.patch).not.toHaveBeenCalled();
    await flush(() =>
      textarea.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }),
      ),
    );
    expect(api.patch).toHaveBeenCalledWith("/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "changed",
      version: 1,
    });
  });

  it("saves with the platform shortcut while mention suggestions are open", async () => {
    api.patch.mockResolvedValue({ data: { ...note, content: "@", version: 2 }, txid: "2" });
    await mount("note-1");
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "@",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.body.querySelector('[role="listbox"]')).toBeTruthy();
    const event = new KeyboardEvent("keydown", {
      key: "Enter",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    await flush(() => textarea.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(true);
    expect(api.patch).toHaveBeenCalledWith("/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "@",
      version: 1,
    });
  });

  it("lets IME keyCode 229 pass through and selects a mention with plain Enter", async () => {
    await mount("note-1");
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "@",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const imeEvent = new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    });
    Object.defineProperty(imeEvent, "keyCode", { value: 229 });
    await flush(() => textarea.dispatchEvent(imeEvent));
    expect(imeEvent.defaultPrevented).toBe(false);
    expect(api.patch).not.toHaveBeenCalled();

    const enter = new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    });
    await flush(() => textarea.dispatchEvent(enter));
    expect(enter.defaultPrevented).toBe(true);
    expect(api.patch).not.toHaveBeenCalled();
    expect(textarea.value).toBe("@channel ");
  });

  it("saves from the title field with the platform shortcut", async () => {
    api.patch.mockResolvedValue({ data: { ...note, title: "Renamed", version: 2 }, txid: "2" });
    await mount("note-1");
    await flush(() => button("編集").click());
    const title = document.body.querySelector("input")!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(title), "value")!.set!.call(
        title,
        "Renamed",
      );
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flush(() =>
      title.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }),
      ),
    );
    expect(api.patch).toHaveBeenCalledWith("/api/channels/channel-1/notes/note-1", {
      title: "Renamed",
      content: note.content,
      version: 1,
    });
  });

  it("copies the selected note direct URL and reports clipboard failures", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    await mount("note-1");
    const copy = Array.from(document.body.querySelectorAll("button")).find(
      (item) => item.getAttribute("aria-label") === "ノートURLをコピー",
    )!;
    await flush(() => copy.click());
    expect(writeText.mock.calls[0]?.[0]).toContain("note=note-1");
    const { toast } = await import("sonner");
    expect(toast.success).toHaveBeenCalled();
    writeText.mockRejectedValueOnce(new Error("denied"));
    await flush(() => copy.click());
    expect(toast.error).toHaveBeenCalled();
  });

  it("autosaves a displayed task checkbox with the selected note version", async () => {
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    api.patch.mockResolvedValue({
      data: { ...note, content: "- [x] do this", version: 2 },
      txid: "2",
    });
    await mount("note-1");
    const checkbox = document.body.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(checkbox.disabled).toBe(false);
    await flush(() => checkbox.click());
    expect(api.patch).toHaveBeenCalledWith("/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "- [x] do this",
      version: 1,
    });
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement)?.checked,
    ).toBe(true);
  });

  it("keeps every task checkbox active during save and serializes a second task edit", async () => {
    let finishFirst!: (value: { data: typeof note; txid: string }) => void;
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] one\n- [ ] two" }],
      txid: null,
    });
    api.patch
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishFirst = resolve;
          }),
      )
      .mockImplementationOnce(async (_path, body) => ({
        data: { ...note, ...body, version: 3 },
        txid: "3",
      }));
    await mount("note-1");
    let boxes = Array.from(
      document.body.querySelectorAll('input[type="checkbox"]'),
    ) as HTMLInputElement[];
    await act(async () => boxes[0]?.click());
    boxes = Array.from(
      document.body.querySelectorAll('input[type="checkbox"]'),
    ) as HTMLInputElement[];
    expect(boxes.every((box) => !box.disabled)).toBe(true);
    await act(async () => boxes[1]?.click());
    expect(
      Array.from(
        document.body.querySelectorAll('input[type="checkbox"]') as NodeListOf<HTMLInputElement>,
      ).every((box) => box.checked),
    ).toBe(true);
    await act(async () =>
      finishFirst({ data: { ...note, content: "- [x] one", version: 2 }, txid: "2" }),
    );
    expect(api.patch).toHaveBeenNthCalledWith(2, "/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "- [x] one\n- [x] two",
      version: 2,
    });
  });

  it("saves nested and empty CRLF tasks from real checkbox clicks with exact Markdown", async () => {
    const content = "- parent\r\n    - [ ] child\r\n- [ ]\r\n";
    api.requestWithTx.mockResolvedValue({ data: [{ ...note, content }], txid: null });
    api.patch.mockImplementation(async (_path, body) => ({
      data: { ...note, ...body, version: 2 },
      txid: "2",
    }));
    await mount("note-1");

    const boxes = Array.from(
      document.body.querySelectorAll('input[type="checkbox"]'),
    ) as HTMLInputElement[];
    expect(boxes).toHaveLength(2);
    expect(boxes.every((checkbox) => !checkbox.disabled)).toBe(true);
    await flush(() => boxes[0]?.click());
    expect(api.patch).toHaveBeenNthCalledWith(1, "/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "- parent\r\n    - [x] child\r\n- [ ]\r\n",
      version: 1,
    });
    const emptyTask = document.body.querySelectorAll(
      'input[type="checkbox"]',
    )[1] as HTMLInputElement;
    expect(emptyTask.disabled).toBe(false);
    await flush(() => emptyTask.click());
    expect(api.patch).toHaveBeenNthCalledWith(2, "/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "- parent\r\n    - [x] child\r\n- [x]\r\n",
      version: 2,
    });
  });

  it("keeps a failed task toggle visible and offers an explicit retry", async () => {
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    api.patch.mockRejectedValueOnce(new Error("offline"));
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(true);
    expect(document.body.querySelector('[role="alert"]')?.textContent).toContain("offline");
    expect(button("再試行")).toBeTruthy();
  });

  it("preserves the failed checkbox draft if creating a new note fails", async () => {
    api.requestWithTx.mockResolvedValue({ data: [{ ...note, content: "- [ ] do this" }] });
    api.patch.mockRejectedValueOnce(new Error("offline"));
    api.post.mockRejectedValueOnce(new Error("create failed"));
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await flush(() =>
      (document.body.querySelector('[aria-label="ノートを作成"]') as HTMLButtonElement).click(),
    );
    expect(document.body.querySelector('[role="alert"]')?.textContent).toContain("create failed");
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(true);
    expect(button("再試行")).toBeTruthy();
  });

  it("preserves the toggled draft and reports a version conflict", async () => {
    api.requestWithTx
      .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }], txid: null })
      .mockResolvedValueOnce({ data: { ...note, content: "- [ ] changed elsewhere", version: 2 } });
    api.patch.mockRejectedValueOnce(
      new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
    );
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(true);
    expect(document.body.querySelector('[role="alert"]')?.textContent).toContain("更新しました");
    await flush(() => button("編集").click());
    expect(document.body.textContent).toContain("最新版の内容: Initial");
    expect(document.body.textContent).toContain("changed elsewhere");
  });

  it("clears the old note conflict when a new note is created", async () => {
    const latest = { ...note, content: "- [ ] changed elsewhere", version: 2 };
    api.requestWithTx
      .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }] })
      .mockResolvedValueOnce({ data: latest });
    api.patch.mockRejectedValueOnce(
      new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
    );
    api.post.mockResolvedValueOnce({ data: { ...note, id: "note-2", title: "New", content: "" } });
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await flush(() =>
      (document.body.querySelector('[aria-label="ノートを作成"]') as HTMLButtonElement).click(),
    );
    expect(onNoteChange).toHaveBeenCalledWith("note-2");
    expect(document.body.textContent).not.toContain("changed elsewhere");
    expect(button("上書きする")).toBeUndefined();
  });

  it("clears the old note latest-fetch retry when a new note is created", async () => {
    api.requestWithTx
      .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }] })
      .mockRejectedValueOnce(new Error("offline"));
    api.patch.mockRejectedValueOnce(
      new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
    );
    api.post.mockResolvedValueOnce({ data: { ...note, id: "note-2", title: "New", content: "" } });
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    expect(button("再試行")).toBeTruthy();
    await flush(() =>
      (document.body.querySelector('[aria-label="ノートを作成"]') as HTMLButtonElement).click(),
    );
    expect(button("再試行")).toBeUndefined();
  });

  it.each(["another note", "messages"])(
    "clears the old conflict and retry when switching to %s",
    async (destination) => {
      const second = { ...note, id: "note-2", title: "Second", content: "Second body" };
      api.requestWithTx
        .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }, second] })
        .mockRejectedValueOnce(new Error("latest unavailable"));
      api.patch.mockRejectedValueOnce(
        new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
      );
      vi.stubGlobal(
        "confirm",
        vi.fn(() => true),
      );
      await mount("note-1");
      await flush(() =>
        (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
      );
      expect(button("再試行")).toBeTruthy();
      await flush(() => button(destination === "messages" ? "メッセージ" : "Second").click());
      expect(button("再試行")).toBeUndefined();
      expect(document.body.textContent).not.toContain("latest unavailable");
      expect(button("上書きする")).toBeUndefined();
    },
  );

  it("saves a conflicted checkbox draft after explicitly choosing to overwrite the latest version", async () => {
    const latest = { ...note, content: "- [ ] changed elsewhere", version: 2 };
    api.requestWithTx
      .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }], txid: null })
      .mockResolvedValueOnce({ data: latest, txid: null })
      .mockResolvedValue({ data: [latest], txid: null });
    api.patch
      .mockRejectedValueOnce(new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }))
      .mockResolvedValueOnce({
        data: { ...latest, content: "- [x] do this", version: 3 },
        txid: "3",
      });
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await flush(() => button("編集").click());
    expect(document.body.textContent).toContain("changed elsewhere");
    await flush(() => button("上書きする").click());
    await flush(() => button("ノートを保存").click());
    expect(api.patch).toHaveBeenLastCalledWith("/api/channels/channel-1/notes/note-1", {
      title: "Initial",
      content: "- [x] do this",
      version: 2,
    });
    expect(button("編集")).toBeTruthy();
    expect(button("再試行")).toBeUndefined();
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement | null)?.checked,
    ).toBe(true);
  });

  it("clears a failed task draft and resumes polling after a successful edit save", async () => {
    api.requestWithTx
      .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }], txid: null })
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue({ data: [{ ...note, content: "- [x] do this", version: 2 }], txid: null });
    api.patch
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce({ data: { ...note, content: "- [x] edited", version: 2 }, txid: "2" });
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea") as HTMLTextAreaElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "- [x] edited",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flush(() => button("ノートを保存").click());
    expect(button("再試行")).toBeUndefined();
    expect(document.body.textContent).toContain("edited");
    expect(api.requestWithTx).toHaveBeenCalledTimes(2);
  });

  it("clears the failed latest-version retry after an edit save succeeds", async () => {
    api.requestWithTx
      .mockResolvedValueOnce({ data: [{ ...note, content: "- [ ] do this" }], txid: null })
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce({
        data: [{ ...note, content: "- [x] do this", version: 2 }],
        txid: null,
      });
    api.patch
      .mockRejectedValueOnce(new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }))
      .mockResolvedValueOnce({ data: { ...note, content: "- [x] saved", version: 2 }, txid: "2" });
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    expect(button("再試行")).toBeTruthy();
    await flush(() => button("編集").click());
    const textarea = document.body.querySelector("textarea") as HTMLTextAreaElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(textarea), "value")!.set!.call(
        textarea,
        "- [x] saved",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flush(() => button("ノートを保存").click());
    expect(button("再試行")).toBeUndefined();
    expect(document.body.textContent).toContain("saved");
  });

  it("discards a failed task draft when edit is cancelled", async () => {
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    api.patch.mockRejectedValueOnce(new Error("offline"));
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await flush(() => button("編集").click());
    await flush(() => button("取消").click());
    expect(button("再試行")).toBeUndefined();
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(false);
  });

  it("discards the task draft when write permission is lost or the note is deleted", async () => {
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    api.patch.mockRejectedValueOnce(new Error("offline"));
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    await mount("note-1", { canWrite: false });
    expect(button("再試行")).toBeUndefined();
    expect(document.body.querySelector("textarea")).toBeNull();
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(false);

    await mount("note-1");
    api.patch.mockRejectedValueOnce(new Error("offline"));
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    api.delete.mockResolvedValue({ data: null, txid: null });
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
    await flush(() =>
      Array.from(document.body.querySelectorAll("button"))
        .find((item) => item.textContent?.trim() === "削除")!
        .click(),
    );
    expect(api.delete).toHaveBeenCalledTimes(1);
    expect(button("再試行")).toBeUndefined();
    expect(document.body.querySelector('input[type="checkbox"]')).toBeNull();
  });

  it("serializes rapid checkbox clicks into one in-flight PATCH", async () => {
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    let resolvePatch!: (value: unknown) => void;
    api.patch.mockReturnValueOnce(new Promise((resolve) => (resolvePatch = resolve)));
    await mount("note-1");
    const checkbox = document.body.querySelector('input[type="checkbox"]') as HTMLInputElement;
    await act(async () => {
      checkbox.click();
      checkbox.click();
    });
    expect(api.patch).toHaveBeenCalledTimes(1);
    resolvePatch({ data: { ...note, content: "- [x] do this", version: 2 }, txid: "2" });
    await flush(() => undefined);
  });

  it("ignores a poll started before a task update when it returns afterward", async () => {
    vi.useFakeTimers();
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    api.patch.mockResolvedValue({
      data: { ...note, content: "- [x] do this", version: 2 },
      txid: "2",
    });
    await mount("note-1");
    let resolvePoll!: (value: unknown) => void;
    api.requestWithTx.mockReturnValueOnce(new Promise((resolve) => (resolvePoll = resolve)));
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    resolvePoll({ data: [{ ...note, content: "- [ ] do this", version: 1 }], txid: null });
    await flush(() => undefined);
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(true);
    vi.useRealTimers();
  });

  it("ignores a poll started during a task update when it returns after saving", async () => {
    vi.useFakeTimers();
    const initial = { ...note, content: "- [ ] do this", version: 1 };
    api.requestWithTx.mockResolvedValue({ data: [initial], txid: null });
    let resolvePatch!: (value: unknown) => void;
    api.patch.mockReturnValueOnce(new Promise((resolve) => (resolvePatch = resolve)));
    await mount("note-1");
    await flush(() =>
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).click(),
    );
    let resolvePoll!: (value: unknown) => void;
    api.requestWithTx.mockReturnValueOnce(new Promise((resolve) => (resolvePoll = resolve)));
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    await flush(() =>
      resolvePatch({ data: { ...initial, content: "- [x] do this", version: 2 }, txid: "2" }),
    );
    await flush(() => resolvePoll({ data: [initial], txid: null }));
    expect(
      (document.body.querySelector('input[type="checkbox"]') as HTMLInputElement).checked,
    ).toBe(true);
    vi.useRealTimers();
  });

  it.each(["create", "delete"])("ignores a poll started during a pending %s", async (action) => {
    vi.useFakeTimers();
    const created = { ...note, id: "note-2", title: "New", content: "", version: 1 };
    api.requestWithTx.mockResolvedValue({ data: [note], txid: null });
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
    let finish!: (value: unknown) => void;
    const pending = new Promise((resolve) => (finish = resolve));
    if (action === "create") api.post.mockReturnValueOnce(pending);
    else api.delete.mockReturnValueOnce(pending);
    await mount("note-1");
    await flush(() => {
      if (action === "create")
        (document.body.querySelector('[aria-label="ノートを作成"]') as HTMLButtonElement).click();
      else
        Array.from(document.body.querySelectorAll("button"))
          .find((b) => b.textContent === "削除")!
          .click();
    });
    let resolvePoll!: (value: unknown) => void;
    api.requestWithTx.mockReturnValueOnce(new Promise((resolve) => (resolvePoll = resolve)));
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    await flush(() => finish({ data: created }));
    await flush(() => resolvePoll({ data: [note], txid: null }));
    const tabs = document.body.querySelector('[role="tablist"]')!.textContent;
    if (action === "create") expect(tabs).toContain("New");
    else expect(tabs).not.toContain("Initial");
    vi.useRealTimers();
  });

  it("does not allow task toggles for read-only or archived channels", async () => {
    api.requestWithTx.mockResolvedValue({
      data: [{ ...note, content: "- [ ] do this" }],
      txid: null,
    });
    await mount("note-1", { canWrite: false });
    const readOnly = document.body.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(readOnly.disabled).toBe(true);
    await mount("note-1", { archived: true });
    const archived = document.body.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(archived.disabled).toBe(true);
    expect(api.patch).not.toHaveBeenCalled();
  });

  it("creates a placeholder note immediately and saves edited raw content with its current version", async () => {
    const created = { ...note, id: "note-2", title: "無題のノート", content: "", version: 1 };
    api.post.mockResolvedValue({ data: created, txid: "1" });
    await mount();
    await flush(() =>
      (
        document.body.querySelector('button[aria-label="ノートを作成"]') as HTMLButtonElement
      ).click(),
    );
    expect(api.post).toHaveBeenCalledWith("/api/channels/channel-1/notes", {
      title: "無題のノート",
      content: "",
    });
    expect(onNoteChange).toHaveBeenCalledWith("note-2");
  });

  it("keeps a dirty draft when creating a note is cancelled", async () => {
    vi.stubGlobal(
      "confirm",
      vi.fn(() => false),
    );
    await mount("note-1");
    await flush(() => button("編集").click());
    const title = document.body.querySelector("input") as HTMLInputElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(title), "value")!.set!.call(
        title,
        "Draft title",
      );
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flush(() =>
      (
        document.body.querySelector('button[aria-label="ノートを作成"]') as HTMLButtonElement
      ).click(),
    );
    expect(window.confirm).toHaveBeenCalled();
    expect(api.post).not.toHaveBeenCalled();
    expect((document.body.querySelector("input") as HTMLInputElement).value).toBe("Draft title");
  });

  it("closes edit mode when saving without changes", async () => {
    await mount("note-1");
    await flush(() => button("編集").click());
    await flush(() => button("ノートを保存").click());
    expect(api.patch).not.toHaveBeenCalled();
    expect(document.body.querySelector("textarea")).toBeNull();
  });

  it("keeps the active draft when polling reports that another user deleted its note", async () => {
    vi.useFakeTimers();
    await mount("note-1");
    await flush(() => button("編集").click());
    const title = document.body.querySelector("input") as HTMLInputElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(title), "value")!.set!.call(
        title,
        "Unsaved draft",
      );
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    api.requestWithTx.mockResolvedValue({ data: [], txid: null });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect((document.body.querySelector("input") as HTMLInputElement).value).toBe("Unsaved draft");
    expect(document.body.querySelector("textarea")).toBeTruthy();
    vi.useRealTimers();
  });

  it("keeps the draft and offers another latest-version fetch after conflict fetch fails", async () => {
    await mount("note-1");
    await flush(() => button("編集").click());
    const title = document.body.querySelector("input") as HTMLInputElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(title), "value")!.set!.call(
        title,
        "Draft",
      );
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    api.patch.mockRejectedValueOnce(
      new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
    );
    api.requestWithTx.mockRejectedValueOnce(new Error("temporary read failure"));
    await flush(() => button("ノートを保存").click());
    expect((document.body.querySelector("input") as HTMLInputElement).value).toBe("Draft");
    expect(document.body.querySelector('[role="alert"]')?.textContent).toContain(
      "temporary read failure",
    );
  });

  it("keeps a draft after a version conflict and only overwrites after the user confirms the latest version", async () => {
    await mount("note-1");
    await flush(() => button("編集").click());
    const title = document.body.querySelector("input") as HTMLInputElement;
    const body = document.body.querySelector("textarea") as HTMLTextAreaElement;
    await flush(() => {
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(title), "value")!.set!.call(
        title,
        "My draft",
      );
      title.dispatchEvent(new Event("input", { bubbles: true }));
      Object.getOwnPropertyDescriptor(Object.getPrototypeOf(body), "value")!.set!.call(
        body,
        "Draft $[x2 text]",
      );
      body.dispatchEvent(new Event("input", { bubbles: true }));
    });
    api.patch.mockRejectedValueOnce(
      new ApiError(409, "Conflict", {}, { method: "PATCH", path: "/note" }),
    );
    api.requestWithTx.mockResolvedValueOnce({
      data: { ...note, title: "Latest", content: "Latest body", version: 2 },
      txid: null,
    });
    await flush(() => button("ノートを保存").click());
    expect((document.body.querySelector("input") as HTMLInputElement).value).toBe("My draft");
    expect(document.body.textContent).toContain("Latest body");
    await flush(() => button("最新版を確認して下書きで上書きする").click());
    api.patch.mockResolvedValueOnce({
      data: { ...note, title: "My draft", content: "Draft $[x2 text]", version: 3 },
      txid: "2",
    });
    await flush(() => button("ノートを保存").click());
    expect(api.patch).toHaveBeenLastCalledWith("/api/channels/channel-1/notes/note-1", {
      title: "My draft",
      content: "Draft $[x2 text]",
      version: 2,
    });
  });

  it("persists a keyboard-accessible move operation as shared note order", async () => {
    api.requestWithTx.mockResolvedValueOnce({
      data: [note, { ...note, id: "note-2", title: "Second" }],
      txid: null,
    });
    await mount();
    await flush(() =>
      (document.body.querySelector('[aria-label="下へ移動 Initial"]') as HTMLButtonElement).click(),
    );
    expect(api.requestWithTx).toHaveBeenCalledWith("/api/channels/channel-1/notes/order", {
      method: "PATCH",
      body: {
        expected_note_ids: ["note-1", "note-2"],
        note_ids: ["note-2", "note-1"],
      },
    });
  });

  it("serializes rapid reorder clicks while the first order update is pending", async () => {
    api.requestWithTx.mockResolvedValueOnce({
      data: [note, { ...note, id: "note-2", title: "Second" }],
      txid: null,
    });
    let finish!: (value: { data: { updated: boolean }; txid: null }) => void;
    api.requestWithTx.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    await mount();
    await act(async () => {
      (document.body.querySelector('[aria-label="下へ移動 Initial"]') as HTMLButtonElement).click();
      (document.body.querySelector('[aria-label="下へ移動 Second"]') as HTMLButtonElement).click();
    });
    expect(api.requestWithTx).toHaveBeenCalledTimes(2);
    expect(
      (document.body.querySelector('[aria-label="下へ移動 Second"]') as HTMLButtonElement).disabled,
    ).toBe(true);
    await flush(() => finish({ data: { updated: true }, txid: null }));
  });
});
