import { describe, expect, it } from "vite-plus/test";

import { classifyScrollChange, isNearBottom, shouldStickToBottom, snapshot } from "./messageScroll";

describe("snapshot", () => {
  it("returns nulls for an empty list", () => {
    expect(snapshot([])).toEqual({ firstId: null, lastId: null });
  });

  it("reads the first and last ids", () => {
    expect(snapshot(["a", "b", "c"])).toEqual({ firstId: "a", lastId: "c" });
  });
});

describe("classifyScrollChange", () => {
  const prev = { firstId: "a", lastId: "b" };

  it("detects the initial load", () => {
    expect(classifyScrollChange({ firstId: null, lastId: null }, prev, new Set(["a", "b"]))).toBe(
      "initial",
    );
  });

  it("detects an append when the previous tail is still present", () => {
    expect(
      classifyScrollChange(prev, { firstId: "a", lastId: "c" }, new Set(["a", "b", "c"])),
    ).toBe("append");
  });

  it("does not treat a removed tail as an append", () => {
    expect(classifyScrollChange(prev, { firstId: "a", lastId: "a" }, new Set(["a"]))).toBe("none");
  });

  it("detects a prepend", () => {
    expect(
      classifyScrollChange(prev, { firstId: "z", lastId: "b" }, new Set(["z", "a", "b"])),
    ).toBe("prepend");
  });

  it("detects a wholesale replace", () => {
    expect(classifyScrollChange(prev, { firstId: "x", lastId: "y" }, new Set(["x", "y"]))).toBe(
      "replace",
    );
  });

  it("reports no change", () => {
    expect(classifyScrollChange(prev, prev, new Set(["a", "b"]))).toBe("none");
    expect(
      classifyScrollChange(
        { firstId: null, lastId: null },
        { firstId: null, lastId: null },
        new Set(),
      ),
    ).toBe("none");
  });
});

describe("shouldStickToBottom", () => {
  it("always sticks on initial load and replace", () => {
    expect(shouldStickToBottom("initial", false)).toBe(true);
    expect(shouldStickToBottom("replace", false)).toBe(true);
  });

  it("sticks on append only when already at the bottom", () => {
    expect(shouldStickToBottom("append", true)).toBe(true);
    expect(shouldStickToBottom("append", false)).toBe(false);
  });

  it("never moves on prepend", () => {
    expect(shouldStickToBottom("prepend", true)).toBe(false);
  });
});

describe("isNearBottom", () => {
  it("is true within the threshold and false beyond it", () => {
    expect(isNearBottom(1000, 1400, 400)).toBe(true);
    expect(isNearBottom(900, 1400, 400)).toBe(false);
    expect(isNearBottom(900, 1400, 400, 120)).toBe(true);
  });
});
