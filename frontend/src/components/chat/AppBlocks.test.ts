import { describe, expect, it } from "vite-plus/test";
import { parseBlocks, hasCardBlocks } from "./AppBlocks";

describe("declarative app content", () => {
  it("only suppresses fallback text for a valid declarative card", () => {
    expect(
      hasCardBlocks(
        JSON.stringify([
          {
            type: "card",
            card: { title: "Ready", fields: [{ title: "Status", value: "Completed" }] },
          },
        ]),
      ),
    ).toBe(true);
    expect(hasCardBlocks('[{"type":"card","card":{"title":3}}]')).toBe(false);
    expect(hasCardBlocks("{")).toBe(false);
    expect(hasCardBlocks('[{"type":"text","text":"hello"}]')).toBe(false);
  });
  it("renders supported content while ignoring executable or malformed blocks", () => {
    expect(parseBlocks([{ type: "text", text: "<script>alert(1)</script>" }])).toEqual([
      { type: "text", text: "<script>alert(1)</script>" },
    ]);
    expect(
      parseBlocks([
        { type: "html", html: "<script>" },
        { type: "select", action_id: "pick", label: "Pick", options: [1] },
        null,
      ]),
    ).toEqual([]);
  });
  it("bounds the number of blocks", () => {
    expect(parseBlocks(Array.from({ length: 51 }, () => ({ type: "text", text: "x" })))).toEqual(
      [],
    );
  });
});
