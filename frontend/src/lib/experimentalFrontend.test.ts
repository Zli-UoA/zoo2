import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { resolveFrontend, supportsComposePath } from "./experimentalFrontend";

beforeEach(() => vi.stubEnv("VITE_EXPERIMENTAL_COMPOSE", "true"));
afterEach(() => vi.unstubAllEnvs());

describe("experimental frontend selection", () => {
  it("defaults to React and requires an explicit selection", () => {
    expect(resolveFrontend("", null)).toBe("react");
    expect(resolveFrontend("", "compose")).toBe("compose");
    expect(resolveFrontend("?frontend=compose", null)).toBe("compose");
    expect(resolveFrontend("?frontend=react", "compose")).toBe("react");
  });

  it("ignores invalid selections and preserves unrelated query parameters", () => {
    expect(resolveFrontend("?frontend=unknown&message=m1", null)).toBe("react");
    expect(resolveFrontend("?message=m1", "compose")).toBe("compose");
  });
});

it("renders settings and Bubble Chat in the selected Compose frontend", () => {
  for (const path of ["/settings", "/settings/account", "/bubble", "/bubble/room"]) {
    expect(resolveFrontend("?frontend=compose", "compose", path)).toBe("compose");
  }
  expect(resolveFrontend("", "compose", "/channels/general")).toBe("compose");
});

for (const enabled of [undefined, "false", "true"]) {
  it(`uses React for retired Compose selections when flag is ${enabled}`, () => {
    vi.stubEnv("VITE_EXPERIMENTAL_COMPOSE", enabled);
    for (const path of ["/", "/settings", "/bubble", "/public/users/u1"]) {
      expect(supportsComposePath(path)).toBe(enabled === "true");
      for (const search of ["", "?frontend=compose", "?frontend=compose&message=m1"]) {
        expect(resolveFrontend(search, "compose", path)).toBe(
          enabled === "true" ? "compose" : "react",
        );
      }
    }
  });
}
