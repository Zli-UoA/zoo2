// @vitest-environment happy-dom
import { expect, it, vi } from "vite-plus/test";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { MarkdownNote, toggleTaskMarker } from "./MarkdownNote";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
import { toast } from "sonner";

it("renders GFM text safely and rejects executable link schemes", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  await act(async () =>
    root.render(
      createElement(MarkdownNote, {
        content:
          "# Title\n\n- [x] done\n\n| A | B |\n| --- | --- |\n| cell | **bold** |\n\n[x](javascript:alert(1)) <img src=x onerror=alert(1)>",
      }),
    ),
  );
  expect(host.querySelector("h1")?.textContent).toBe("Title");
  expect((host.querySelector("input[type=checkbox]") as HTMLInputElement | null)?.checked).toBe(
    true,
  );
  expect(host.querySelector("table")).not.toBeNull();
  expect(host.querySelector("a[href^=javascript]")).toBeNull();
  expect(host.querySelector("img")).toBeNull();
  expect(host.querySelector("[onerror]")).toBeNull();
  root.unmount();
});

it("removes only task-list bullets while preserving ordinary and nested list markers", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  await act(async () =>
    root.render(
      createElement(MarkdownNote, {
        content: "- [ ] task\n- regular\n  - nested\n\n1. [x] ordered task",
      }),
    ),
  );
  const taskItems = Array.from(host.querySelectorAll("li")).filter((item) =>
    item.querySelector('input[type="checkbox"]'),
  );
  expect(taskItems).toHaveLength(2);
  expect(taskItems.every((item) => item.classList.contains("task-list-item"))).toBe(true);
  expect(host.querySelector("ul")?.classList.contains("contains-task-list")).toBe(true);
  expect(host.querySelectorAll("li")).toHaveLength(4);
  root.unmount();
});

it("copies code block source and reports clipboard success or failure", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  await act(async () =>
    root.render(createElement(MarkdownNote, { content: "```ts\nconst x = 1;\n```" })),
  );
  const copy = host.querySelector(".markdown-code-copy") as HTMLButtonElement;
  expect(copy).toBeTruthy();
  await act(async () => copy.click());
  expect(writeText).toHaveBeenCalledWith("const x = 1;\n");
  expect(toast.success).toHaveBeenCalled();
  writeText.mockRejectedValueOnce(new Error("denied"));
  await act(async () => copy.click());
  expect(toast.error).toHaveBeenCalled();
  root.unmount();
});

it("changes only the AST-identified GFM task marker, including CRLF and nested Unicode text", () => {
  const source =
    "# café 🐈\r\n\r\n- [ ] same [x] text\r\n  - [x] same task 🐈\r\n\r\n```md\r\n- [ ] code\r\n```\r\n";
  const nestedTaskStart = source.indexOf("[x] same task") + 1;
  expect(toggleTaskMarker(source, nestedTaskStart, false)).toBe(
    source.replace("[x] same task", "[ ] same task"),
  );
  expect(toggleTaskMarker(source, source.indexOf("[ ] same") + 1, true)).toBe(
    source.replace("- [ ] same", "- [x] same"),
  );
  expect(toggleTaskMarker(source, source.indexOf("text"), true)).toBe(source);
});

it("toggles valid four-space nested tasks and empty CRLF tasks at their AST source offsets", () => {
  const nested = "- parent\n    - [ ] child\n";
  const nestedState = nested.indexOf("[ ]") + 1;
  expect(toggleTaskMarker(nested, nestedState, true)).toBe("- parent\n    - [x] child\n");

  const emptyCrLf = "- [ ]\r\n";
  const emptyState = emptyCrLf.indexOf("[ ]") + 1;
  expect(toggleTaskMarker(emptyCrLf, emptyState, true)).toBe("- [x]\r\n");
});

it("does not treat an escaped empty marker as an editable task", async () => {
  const source = "- \\[ \\]";
  expect(toggleTaskMarker(source, source.indexOf("[") + 1, true)).toBe(source);
  const host = document.createElement("div");
  const root = createRoot(host);
  await act(async () =>
    root.render(createElement(MarkdownNote, { content: source, taskCheckboxes: true })),
  );
  expect(host.querySelector('input[type="checkbox"]')).toBeNull();
  root.unmount();
});

it("only toggles AST task checkboxes after quoted fences, never code lookalikes or duplicate positions", () => {
  const source =
    "> ```md\n> - [ ] quoted fence code\n> ```\n> - [ ] real task\n\n```md\n- [ ] fenced code\n```\n";
  const realTaskState = source.indexOf("[ ] real") + 1;
  expect(toggleTaskMarker(source, realTaskState, true)).toBe(
    source.replace("[ ] real task", "[x] real task"),
  );
  expect(toggleTaskMarker(source, source.indexOf("[ ] quoted") + 1, true)).toBe(source);
  expect(toggleTaskMarker(source, source.indexOf("[ ] fenced") + 1, true)).toBe(source);
  expect(toggleTaskMarker(source, realTaskState + 1, true)).toBe(source);

  const duplicates = "- [ ] same\n- [ ] same\n";
  const secondDuplicate = duplicates.lastIndexOf("[ ]") + 1;
  expect(toggleTaskMarker(duplicates, secondDuplicate, true)).toBe("- [ ] same\n- [x] same\n");
});

it("toggles blockquoted task markers at their exact source line, including nested quotes", () => {
  const source = "> - [ ] quoted\n> > 3. [x] nested 🐈\n";
  expect(toggleTaskMarker(source, source.indexOf("[ ] quoted") + 1, true)).toBe(
    "> - [x] quoted\n> > 3. [x] nested 🐈\n",
  );
  expect(toggleTaskMarker(source, source.indexOf("[x] nested") + 1, false)).toBe(
    "> - [ ] quoted\n> > 3. [ ] nested 🐈\n",
  );
  expect(toggleTaskMarker(source, source.indexOf("nested"), false)).toBe(source);
});

it("does not modify task-looking text in code blocks or when the source position is wrong", () => {
  const source = "```md\n- [ ] fenced\n```\n    - [ ] indented\n- [ ] actual\n";
  expect(toggleTaskMarker(source, source.indexOf("[ ] fenced") + 1, true)).toBe(source);
  expect(toggleTaskMarker(source, source.indexOf("[ ] indented") + 1, true)).toBe(source);
  expect(toggleTaskMarker(source, source.indexOf("actual") - 7, true)).toBe(source);
  expect(toggleTaskMarker(source, source.indexOf("[ ] actual") + 1, true)).toBe(
    source.replace("- [ ] actual", "- [x] actual"),
  );
});

it("exposes task controls only when enabled and forwards a task checkbox operation", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const onTaskToggle = vi.fn();
  const source = "- [ ] first task\n- [x] second task";
  await act(async () =>
    root.render(
      createElement(MarkdownNote, { content: source, taskCheckboxes: true, onTaskToggle }),
    ),
  );
  const boxes = Array.from(host.querySelectorAll("input[type=checkbox]")) as HTMLInputElement[];
  expect(boxes[0]?.disabled).toBe(false);
  expect(boxes[0]?.getAttribute("aria-label")).toBe("first task");
  await act(async () => boxes[0]?.click());
  expect(onTaskToggle).toHaveBeenCalledWith(expect.any(Number), true);
  root.unmount();
});

it("keeps the blockquote checkbox position actionable through the Markdown AST", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const source = "> - [ ] quoted task";
  const onTaskToggle = vi.fn((offset: number) => toggleTaskMarker(source, offset, true));
  await act(async () =>
    root.render(
      createElement(MarkdownNote, { content: source, taskCheckboxes: true, onTaskToggle }),
    ),
  );
  const checkbox = host.querySelector('input[type="checkbox"]') as HTMLInputElement;
  expect(checkbox.disabled).toBe(false);
  await act(async () => checkbox.click());
  expect(onTaskToggle).toHaveReturnedWith("> - [x] quoted task");
  root.unmount();
});

it("enables only AST task checkboxes and forwards the state-character source offset", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const source = "- parent\n    - [ ] child\n\n```md\n- [ ] code\n```\n";
  const onTaskToggle = vi.fn();
  await act(async () =>
    root.render(
      createElement(MarkdownNote, { content: source, taskCheckboxes: true, onTaskToggle }),
    ),
  );
  const boxes = Array.from(host.querySelectorAll('input[type="checkbox"]')) as HTMLInputElement[];
  expect(boxes).toHaveLength(1);
  expect(boxes[0]?.disabled).toBe(false);
  await act(async () => boxes[0]?.click());
  expect(onTaskToggle).toHaveBeenCalledWith(source.indexOf("[ ]") + 1, true);
  root.unmount();
});

it("preserves formatted GFM nodes while marking only visible text mentions", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  await act(async () =>
    root.render(
      createElement(MarkdownNote, {
        content: [
          "# Heading **bold @heading**",
          "",
          "Paragraph *italic @para* and [link @label](https://example.test/@destination) with `@code`.",
          "",
          "- **bold @item**",
          "  - nested *italic @nested*",
          "",
          "| Cell | Value |",
          "| --- | --- |",
          "| @table | ~~@strike~~ |",
          "",
          "Escaped \\@escape &commat;entity <b>@rawhtml</b>",
        ].join("\n"),
      }),
    ),
  );

  expect(host.querySelector("h1 strong")?.textContent).toContain("bold @heading");
  expect(host.querySelector("p em")?.textContent).toContain("italic @para");
  expect(host.querySelector("ul > li strong")?.textContent).toContain("bold @item");
  expect(host.querySelector("ul ul li em")?.textContent).toContain("italic @nested");
  expect(host.querySelector("table tbody tr td")?.textContent).toBe("@table");
  expect(host.querySelector("td del")?.textContent).toBe("@strike");
  expect(host.querySelector('a[href="https://example.test/@destination"]')?.textContent).toBe(
    "link @label",
  );
  expect(host.querySelector("code")?.textContent).toBe("@code");
  expect(host.querySelector("a span.font-medium")?.textContent).toBe("@label");
  expect(host.querySelector("td span.font-medium")?.textContent).toBe("@table");
  expect(host.querySelectorAll("span.font-medium").length).toBe(10);
  expect(host.querySelectorAll("span.font-medium")[7]?.textContent).toBe("@escape");
  expect(host.querySelectorAll("span.font-medium")[8]?.textContent).toBe("@entity");
  expect(host.querySelectorAll("span.font-medium")[9]?.textContent).toBe("@rawhtml");
  expect(host.querySelectorAll("span.font-medium")[0]?.textContent).toBe("@heading");
  expect(host.querySelector("td del span.font-medium")?.textContent).toBe("@strike");
  expect(host.querySelector(".markdown-note")).not.toBeNull();
  root.unmount();
});
