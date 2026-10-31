import { describe, expect, it } from "vite-plus/test";
import { parseBlocks } from "./AppBlocks";

describe("declarative app content", () => {
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
