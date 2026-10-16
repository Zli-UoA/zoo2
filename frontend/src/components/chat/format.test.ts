import { describe, expect, it } from "vite-plus/test";

import { initials } from "./format";

describe("initials", () => {
  it("uses the first letter of one or two words", () => {
    expect(initials("Alice")).toBe("A");
    expect(initials("Alice Smith")).toBe("AS");
    expect(initials("Alice B. Smith")).toBe("AB");
  });

  it("keeps the first character of a single japanese name", () => {
    expect(initials("わたし")).toBe("わ");
    expect(initials("山田 太郎")).toBe("山太");
  });

  it("falls back to ? for empty input", () => {
    expect(initials("")).toBe("?");
    expect(initials("   ")).toBe("?");
  });
});
