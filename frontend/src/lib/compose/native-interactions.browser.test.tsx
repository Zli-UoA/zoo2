import { afterEach, describe, expect, it, vi } from "vite-plus/test";
import { runNativeInteractions } from "../../../.storybook/native-interactions";

const frames: HTMLIFrameElement[] = [];
function frame() {
  const result = document.createElement("iframe");
  document.body.append(result);
  frames.push(result);
  return result;
}
afterEach(() => frames.splice(0).forEach((item) => item.remove()));

describe("native Storybook browser interaction driver", () => {
  it("checks callback paths, argument values and call counts", async () => {
    const target = frame();
    const onSend = vi.fn();
    onSend("hello", undefined, { enabled: true });
    await runNativeInteractions(
      target,
      [
        {
          action: "callback",
          name: "actions.onSend",
          count: 1,
          args: ["hello", { __zooUndefined: true }, { enabled: true }],
        },
        { action: "callback", name: "unused", count: 0 },
      ],
      { actions: { onSend }, unused: vi.fn() },
    );
  });

  it("verifies all matched controls, counts, attributes, exact values and dimensions", async () => {
    const target = frame();
    target.contentDocument!.body.innerHTML =
      '<div role="checkbox" aria-checked="true" data-index="1" style="width:20px;height:20px">A</div><div role="checkbox" aria-checked="true" data-index="2" style="width:20px;height:20px">B</div><input aria-label="Result" value="done">';
    await runNativeInteractions(target, [
      {
        action: "wait",
        role: "checkbox",
        count: 2,
        all: true,
        checked: true,
        minAttributes: { "data-index": 1 },
        width: 20,
        height: 20,
        minWidth: 19,
        maxHeight: 21,
      },
      {
        action: "wait",
        role: "checkbox",
        index: 0,
        attributes: { "aria-checked": "true", missing: null },
      },
      { action: "wait", role: "textbox", name: "Result", valueExact: "done" },
      { action: "wait", role: "alert", count: 0 },
    ]);
  });

  it("finds controls inside shadow roots and waits for their changed state", async () => {
    const target = frame();
    const host = target.contentDocument!.createElement("div");
    target.contentDocument!.body.append(host);
    const root = host.attachShadow({ mode: "open" });
    root.innerHTML = '<button aria-pressed="false" aria-busy="true">Toggle</button>';
    const button = root.querySelector("button")!;
    button.onclick = () => {
      button.setAttribute("aria-pressed", "true");
      button.setAttribute("aria-busy", "false");
    };
    await runNativeInteractions(target, [
      { action: "click", role: "button", name: "Toggle" },
      { action: "wait", role: "button", name: "Toggle", pressed: true, busy: false },
    ]);
    expect(button.getAttribute("aria-pressed")).toBe("true");
  });

  it("uses an explicit index or nearest named row rather than an arbitrary first match", async () => {
    const target = frame();
    target.contentDocument!.body.innerHTML =
      "<section><span>Alice</span><button>Change</button></section><section><span>Bob</span><button>Change</button></section>";
    const buttons = target.contentDocument!.querySelectorAll("button");
    const called: number[] = [];
    buttons.forEach((button, index) => {
      button.onclick = () => called.push(index);
    });
    await runNativeInteractions(target, [
      { action: "click", role: "button", name: "Change", within: { text: "Bob" } },
      { action: "click", role: "button", name: "Change", index: 0 },
    ]);
    expect(called).toEqual([1, 0]);
  });

  it("types through the shadow browser input owner and distinguishes append from fill", async () => {
    const target = frame();
    const host = target.contentDocument!.createElement("div");
    target.contentDocument!.body.append(host);
    const root = host.attachShadow({ mode: "open" });
    root.innerHTML =
      '<div role="textbox" tabindex="0" aria-label="Search">before </div><textarea aria-label="Search" tabindex="-1" style="opacity:0;position:absolute;width:1px;height:1px;top:calc(var(--compose-internal-web-backing-input-top) * 1px)"></textarea>';
    const proxy = root.querySelector<HTMLElement>('[role="textbox"]')!;
    const input = root.querySelector("textarea")!;
    input.value = "before ";
    proxy.onfocus = () => {
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    };
    const events: string[] = [];
    const beforeValues: string[] = [];
    input.onbeforeinput = () => {
      beforeValues.push(input.value);
    };
    input.oninput = (event) => {
      events.push((event as InputEvent).inputType);
      proxy.textContent = input.value;
    };
    await runNativeInteractions(target, [
      { action: "type", role: "textbox", name: "Search", text: "hello" },
      { action: "wait", role: "textbox", name: "Search", value: "before hello" },
      { action: "type", role: "textbox", name: "Search", value: "replacement" },
    ]);
    expect(input.value).toBe("replacement");
    expect(proxy.textContent).toBe("replacement");
    expect(events).toEqual(["insertText", "insertText"]);
    expect(beforeValues).toEqual(["before ", "before hello"]);
  });

  it("preserves projected multiline values when typing through the browser editor", async () => {
    const target = frame();
    const doc = target.contentDocument!;
    doc.body.innerHTML =
      '<div role="textbox" tabindex="0" aria-label="Composer"></div><textarea tabindex="-1" style="top:calc(var(--compose-internal-web-backing-input-top) * 1px)"></textarea>';
    const proxy = doc.querySelector<HTMLElement>('[role="textbox"]')!;
    const input = doc.querySelector("textarea")!;
    proxy.onfocus = () => input.focus();
    input.oninput = () => {
      // ComposeWebSemanticsListener uses innerText, creating BR nodes for newlines.
      proxy.innerText = input.value;
    };
    const value = "1行目\n2行目\n3行目 ";
    await runNativeInteractions(target, [
      { action: "type", role: "textbox", name: "Composer", text: value },
      { action: "wait", role: "textbox", name: "Composer", valueExact: value },
    ]);
    expect(input.value).toBe(value);
    expect(proxy.innerText.trimEnd()).toBe(value.trimEnd());
    expect(proxy.textContent).not.toBe(value);
  });

  it("selects a real HTML option by label and commits its actual value", async () => {
    const target = frame();
    target.contentDocument!.body.innerHTML =
      '<select aria-label="Language"><option value="ja">日本語</option><option value="en">English</option></select>';
    let changes = 0;
    const select = target.contentDocument!.querySelector("select")!;
    select.onchange = () => changes++;
    await runNativeInteractions(target, [
      { action: "select", role: "combobox", name: "Language", value: "English" },
    ]);
    expect(select.value).toBe("en");
    expect(changes).toBe(1);
  });

  it("finishes selecting a controlled field that retains its old value", async () => {
    const target = frame();
    target.contentDocument!.body.innerHTML =
      '<select aria-label="Language"><option value="ja">日本語</option><option value="en">English</option></select>';
    const select = target.contentDocument!.querySelector("select")!;
    let calls = 0;
    select.onchange = () => {
      calls++;
      select.value = "ja";
    };
    await runNativeInteractions(target, [
      { action: "select", role: "combobox", name: "Language", value: "English" },
    ]);
    expect(select.value).toBe("ja");
    expect(calls).toBe(1);
  });

  it("finishes a native option click when locale changes the control's accessible name", async () => {
    const target = frame();
    const doc = target.contentDocument!;
    const control = doc.createElement("button");
    control.setAttribute("role", "combobox");
    control.setAttribute("aria-label", "言語");
    control.setAttribute("aria-valuetext", "日本語");
    doc.body.append(control);
    let calls = 0;
    control.onclick = () => {
      const option = doc.createElement("button");
      option.setAttribute("role", "option");
      option.textContent = "English";
      option.onclick = () => {
        calls++;
        control.setAttribute("aria-label", "Language");
        control.setAttribute("aria-valuetext", "English");
        option.remove();
      };
      doc.body.append(option);
    };
    await runNativeInteractions(target, [
      { action: "select", role: "combobox", name: "言語", value: "English" },
    ]);
    expect(control.getAttribute("aria-label")).toBe("Language");
    expect(control.getAttribute("aria-valuetext")).toBe("English");
    expect(calls).toBe(1);
  });

  it.each([false, true])(
    "sends Escape to the current focus owner (removed trigger=%s)",
    async (removeTrigger) => {
      const target = frame();
      const doc = target.contentDocument!;
      const open = doc.createElement("button");
      open.textContent = "Open";
      doc.body.append(open);
      open.onclick = () => {
        if (removeTrigger) open.remove();
        const dialog = doc.createElement("div");
        dialog.setAttribute("role", "dialog");
        const close = doc.createElement("button");
        close.textContent = "Close";
        dialog.append(close);
        dialog.onkeydown = (event) => {
          if (event.key === "Escape") dialog.remove();
        };
        doc.body.append(dialog);
        close.focus();
      };
      await runNativeInteractions(target, [
        { action: "click", role: "button", name: "Open" },
        { action: "key", key: "Escape" },
        { action: "wait", role: "dialog", state: "hidden" },
      ]);
      expect(doc.querySelector('[role="dialog"]')).toBeNull();
    },
  );

  it("pastes original file bytes at an unfocused logical textbox without creating a browser editor", async () => {
    const target = frame();
    const doc = target.contentDocument!;
    doc.body.innerHTML = '<div role="textbox" tabindex="0" aria-label="Composer"></div>';
    const proxy = doc.querySelector<HTMLElement>('[role="textbox"]')!;
    const before = doc.activeElement;
    const received: File[] = [];
    let focusEvents = 0;
    proxy.onfocus = () => focusEvents++;
    proxy.addEventListener("paste", (event) => {
      received.push(...Array.from((event as ClipboardEvent).clipboardData!.files));
      event.preventDefault();
    });
    await runNativeInteractions(target, [
      {
        action: "paste",
        role: "textbox",
        name: "Composer",
        file: { name: "notes.pdf", type: "application/pdf", base64: "AQIDBAU=" },
      },
    ]);
    expect(focusEvents).toBe(0);
    expect(doc.activeElement).toBe(before);
    expect(received).toHaveLength(1);
    expect(received[0].name).toBe("notes.pdf");
    expect(received[0].type).toBe("application/pdf");
    expect([...new Uint8Array(await received[0].arrayBuffer())]).toEqual([1, 2, 3, 4, 5]);
    expect(doc.querySelector("textarea,input")).toBeNull();
  });

  it("rejects unsupported operations instead of silently skipping them", async () => {
    await expect(runNativeInteractions(frame(), [{ action: "unsupported" }])).rejects.toThrow(
      "Unsupported native interaction",
    );
  });
});
