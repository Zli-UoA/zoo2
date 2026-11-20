import { describe, expect, it } from "vite-plus/test";
import all from "../../../../../crates/zoo-components/fixtures/valid/message-all-display.json";
import interactive from "../../../../../crates/zoo-components/fixtures/valid/message-interactive.json";
import modal from "../../../../../crates/zoo-components/fixtures/valid/modal-inputs.json";
import { T, accentColor, hidesContent, isInput, parseComponents, safeUrl } from "./parse";

describe("parseComponents", () => {
  it("reads the shared fixtures that the backend validates", () => {
    for (const fixture of [all, interactive, modal]) {
      const doc = parseComponents(JSON.stringify(fixture.document));
      expect(doc?.components.length).toBe(fixture.document.components.length);
    }
    expect(hidesContent(parseComponents(all.document))).toBe(true);
    expect(hidesContent(parseComponents(interactive.document))).toBe(false);
  });

  it("accepts Discord names and drops malformed children", () => {
    const doc = parseComponents({
      components: [
        { type: "container", components: [{ type: "text_display", content: "hi" }, 42, null] },
      ],
    });
    expect(doc?.components[0]?.type).toBe(T.Container);
    expect(doc?.components[0]?.components).toEqual([{ type: T.TextDisplay, content: "hi" }]);
  });

  it("falls back to the body for broken or empty documents", () => {
    for (const value of [null, "", "not json", "[]", { components: [] }, { components: "x" }])
      expect(parseComponents(value)).toBeNull();
  });

  it("keeps unknown types so their text can be shown", () => {
    const doc = parseComponents({
      components: [{ type: 1000, original_type: "99", text: "kept" }, { type: 4242 }],
    });
    expect(doc?.components.map((c) => c.type)).toEqual([1000, 4242]);
  });

  it("only allows https media and links", () => {
    expect(safeUrl("https://example.com/a.png")).toBe("https://example.com/a.png");
    for (const url of ["javascript:alert(1)", "http://example.com", "https://u:p@example.com", 3])
      expect(safeUrl(url)).toBeNull();
    expect(accentColor(0x2eb886)).toBe("#2eb886");
    expect(accentColor(-1)).toBeUndefined();
  });

  it("knows which components carry input values", () => {
    expect(isInput({ type: T.TextInput, custom_id: "a" })).toBe(true);
    expect(isInput({ type: T.Button, custom_id: "a" })).toBe(false);
    expect(isInput({ type: T.TextInput })).toBe(false);
  });
});
