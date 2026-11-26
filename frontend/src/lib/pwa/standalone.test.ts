import { describe, expect, it } from "vite-plus/test";

import { isStandaloneDisplay } from "./standalone";

describe("isStandaloneDisplay", () => {
  it("display-mode: standalone ならアプリ", () => {
    expect(isStandaloneDisplay({ matchMedia: () => ({ matches: true }), navigator: {} })).toBe(
      true,
    );
  });

  it("iOS の navigator.standalone もアプリ", () => {
    expect(
      isStandaloneDisplay({
        matchMedia: () => ({ matches: false }),
        navigator: { standalone: true },
      }),
    ).toBe(true);
  });

  it("どちらでもなければタブ。matchMedia がなくても投げない", () => {
    expect(isStandaloneDisplay({ matchMedia: () => ({ matches: false }), navigator: {} })).toBe(
      false,
    );
    expect(isStandaloneDisplay({ navigator: undefined })).toBe(false);
  });
});
