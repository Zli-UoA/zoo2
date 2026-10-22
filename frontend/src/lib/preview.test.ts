import { afterEach, describe, expect, it, vi } from "vite-plus/test";
import { resolveConfig } from "./config";
import { PREVIEW_USERS, previewUserId } from "./preview";

afterEach(() => vi.unstubAllGlobals());

describe("preview user selection", () => {
  it("uses the same selected user for REST and Shape configuration", () => {
    vi.stubGlobal("window", { sessionStorage: { getItem: () => PREVIEW_USERS[1].id } });
    expect(previewUserId()).toBe(PREVIEW_USERS[1].id);
    expect(resolveConfig({ VITE_PREVIEW_AUTH: "true" }).devUserId).toBe(PREVIEW_USERS[1].id);
    expect(resolveConfig({ DEV: false }).devUserId).toBeNull();
  });

  it("falls back to the seeded admin for invalid storage or unavailable browser storage", () => {
    vi.stubGlobal("window", { sessionStorage: { getItem: () => "unseeded-user" } });
    expect(previewUserId()).toBe(PREVIEW_USERS[0].id);
    vi.stubGlobal("window", {
      sessionStorage: {
        getItem: () => {
          throw new Error("Storage blocked");
        },
      },
    });
    expect(previewUserId()).toBe(PREVIEW_USERS[0].id);
  });
});
