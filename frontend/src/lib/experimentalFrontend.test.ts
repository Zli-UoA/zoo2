import { describe, expect, it } from "vite-plus/test";

import { resolveFrontend } from "./experimentalFrontend";

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
