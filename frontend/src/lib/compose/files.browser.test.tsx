import { afterEach, describe, expect, it } from "vite-plus/test";
import { createComposeFiles } from "./files";

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
  const owner = frame.contentDocument!;
  const viewport = owner.createElement("div");
  owner.body.append(viewport);
  const editing = owner.createElement("div").attachShadow({ mode: "open" });
  viewport.append(editing.host);
  const input = owner.createElement("textarea");
  editing.append(input);
  const files = createComposeFiles(viewport);
  cleanups.push(() => files.close());
  const deliveries: { origin: string; files: { id: string; name: string }[] }[] = [];
  files.target("composer", JSON.stringify({ enabled: true }), (raw) => {
    deliveries.push(JSON.parse(raw));
    return true;
  });
  files.focus("composer");
  const paste = (target: HTMLElement = input) => {
    const clipboardData = new DataTransfer();
    clipboardData.items.add(
      new File([new Uint8Array([1, 2, 3, 4])], "photo.png", { type: "image/png" }),
    );
    const event = new ClipboardEvent("paste", {
      clipboardData,
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    target.dispatchEvent(event);
    return event;
  };
  return { owner, viewport, files, deliveries, paste };
}

describe("Compose browser file ownership", () => {
  it("receives iframe shadow-input paste and retains original bytes until Kotlin releases them", async () => {
    const { owner, viewport, files, deliveries, paste } = fixture();
    expect(viewport.querySelector("input[type=file]")?.ownerDocument).toBe(owner);
    expect(paste().defaultPrevented).toBe(true);
    expect(deliveries).toHaveLength(1);
    expect(deliveries[0].origin).toBe("paste");
    const file = deliveries[0].files[0];
    expect(file.name).toBe("photo.png");
    expect(await files.invoke("fileRead", file.id)).toBe("AQIDBA==");
    files.release(file.id);
    await expect(files.invoke("fileRead", file.id)).rejects.toThrow("File was released");
  });

  it("routes an unfocused logical textbox by Kotlin bounds without moving focus", async () => {
    const { owner, viewport, files, deliveries, paste } = fixture();
    const logical = owner.createElement("div");
    logical.setAttribute("role", "textbox");
    logical.style.cssText = "width:200px;height:32px";
    viewport.append(logical);
    const rect = logical.getBoundingClientRect();
    const state = {
      enabled: true,
      bounds: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
    };
    const receive = (raw: string) => {
      deliveries.push(JSON.parse(raw));
      return true;
    };
    files.target("composer", JSON.stringify(state), receive);
    files.focus("");
    const before = owner.activeElement;
    expect(paste(logical).defaultPrevented).toBe(true);
    expect(owner.activeElement).toBe(before);
    expect(deliveries).toHaveLength(1);
    expect(await files.invoke("fileRead", deliveries[0].files[0].id)).toBe("AQIDBA==");
    files.target("overlapping", JSON.stringify(state), () => {
      throw new Error("ambiguous target called");
    });
    expect(paste(logical).defaultPrevented).toBe(false);
    files.focus("composer");
    expect(paste(logical).defaultPrevented).toBe(true);
    expect(deliveries).toHaveLength(2);
  });

  it("does not intercept disabled, forgotten, unfocused or disposed targets", () => {
    const { files, deliveries, paste } = fixture();
    files.focus("");
    expect(paste().defaultPrevented).toBe(false);
    files.focus("composer");
    files.target("composer", JSON.stringify({ enabled: false }), () => {
      throw new Error("disabled target called");
    });
    expect(paste().defaultPrevented).toBe(false);
    files.forget("composer");
    expect(paste().defaultPrevented).toBe(false);
    files.close();
    expect(paste().defaultPrevented).toBe(false);
    expect(deliveries).toHaveLength(0);
  });
});
