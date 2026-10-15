import { describe, expect, it } from "vite-plus/test";

import { getCollections, isBrowser } from "./collections";

describe("collections", () => {
  it("refuses to create Electric collections outside the browser", () => {
    expect(isBrowser()).toBe(false);
    expect(() => getCollections()).toThrow(/browser/);
  });
});
