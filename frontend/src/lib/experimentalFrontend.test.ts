import { describe, expect, it } from "vite-plus/test";

import { resolveFrontend } from "./experimentalFrontend";

describe("experimental frontend selection", () => {
  it("keeps React in disabled deployments regardless of stale preferences or query", () => {
    expect(resolveFrontend(false, "?frontend=compose", "compose")).toBe("react");
  });

  it("defaults to React and requires an explicit selection", () => {
    expect(resolveFrontend(true, "", null)).toBe("react");
    expect(resolveFrontend(true, "", "compose")).toBe("compose");
    expect(resolveFrontend(true, "?frontend=compose", null)).toBe("compose");
    expect(resolveFrontend(true, "?frontend=react", "compose")).toBe("react");
  });

  it("ignores invalid selections and preserves unrelated query parameters", () => {
    expect(resolveFrontend(true, "?frontend=unknown&message=m1", null)).toBe("react");
    expect(resolveFrontend(true, "?message=m1", "compose")).toBe("compose");
  });
});
