import { describe, expect, it } from "vite-plus/test";

import { resolveConfig } from "../config";
import { getCollections, isBrowser, shapeAuthSource } from "./collections";

describe("collections", () => {
  it("refuses to create Electric collections outside the browser", () => {
    expect(isBrowser()).toBe(false);
    expect(() => getCollections()).toThrow(/browser/);
  });
});

describe("shapeAuthSource", () => {
  it("disables auth headers in direct mode", () => {
    expect(shapeAuthSource(resolveConfig({ DEV: true }))).toEqual({ auth: false });
  });

  it("forwards the dev user in proxy mode", () => {
    const config = resolveConfig({
      DEV: true,
      VITE_SHAPE_MODE: "proxy",
      VITE_DEV_USER_ID: "dev-user-1",
    });
    expect(shapeAuthSource(config)).toEqual({ devUserId: "dev-user-1" });
  });
});
