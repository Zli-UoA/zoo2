import { afterEach, describe, expect, it } from "vite-plus/test";
import { noAutofill } from "../../components/ui/autofill";
import { applyComposeInputHints } from "./input-hints";

const cleanups: (() => void)[] = [];
afterEach(() =>
  cleanups
    .splice(0)
    .reverse()
    .forEach((cleanup) => cleanup()),
);

function fixture() {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  cleanups.push(() => frame.remove());
  const documentOwner = frame.contentDocument!;
  const host = documentOwner.createElement("div");
  documentOwner.body.append(host);
  const cleanup = applyComposeInputHints(host);
  cleanups.push(cleanup);
  return { documentOwner, host, cleanup };
}

describe("Compose browser input hints", () => {
  it("applies shared hints to late nested shadow editing owners and keeps other UI untouched", async () => {
    const { documentOwner, host } = fixture();
    const outside = documentOwner.createElement("input");
    documentOwner.body.append(outside);
    const first = host.attachShadow({ mode: "open" });
    const inner = documentOwner.createElement("div");
    first.append(inner);
    const nested = inner.attachShadow({ mode: "open" });
    const editing = documentOwner.createElement("textarea");
    nested.append(editing);
    await expect.poll(() => editing.getAttribute("autocomplete")).toBe(noAutofill.autoComplete);
    for (const [key, value] of Object.entries(noAutofill))
      expect(editing.getAttribute(key === "autoComplete" ? "autocomplete" : key)).toBe(
        String(value),
      );
    expect(outside.hasAttribute("data-1p-ignore")).toBe(false);
    editing.setAttribute("autocomplete", "on");
    await expect.poll(() => editing.getAttribute("autocomplete")).toBe("off");
  });

  it("disconnects on cleanup and does not repeatedly set unchanged attributes", async () => {
    const { documentOwner, host, cleanup } = fixture();
    const input = documentOwner.createElement("input");
    const records: MutationRecord[] = [];
    const watcher = new MutationObserver((changes) => records.push(...changes));
    watcher.observe(input, { attributes: true });
    cleanups.push(() => watcher.disconnect());
    host.append(input);
    await expect.poll(() => input.hasAttribute("data-1p-ignore")).toBe(true);
    expect(records).toHaveLength(Object.keys(noAutofill).length);
    cleanup();
    input.setAttribute("autocomplete", "on");
    const shadow = host.attachShadow({ mode: "open" });
    const after = documentOwner.createElement("textarea");
    shadow.append(after);
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    expect(input.getAttribute("autocomplete")).toBe("on");
    expect(after.hasAttribute("data-1p-ignore")).toBe(false);
  });
});
