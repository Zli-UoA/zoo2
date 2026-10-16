import { describe, expect, it } from "vite-plus/test";

import {
  DEFAULT_THEME,
  THEME_STORAGE_KEY,
  applyThemeClass,
  isTheme,
  readStoredTheme,
  resolveTheme,
  themeInitScript,
  type ThemeRoot,
} from "./theme";

function fakeStorage(value: string | null): Pick<Storage, "getItem"> {
  return { getItem: () => value };
}

function fakeRoot(): ThemeRoot & { classes: Set<string> } {
  const classes = new Set<string>();
  return {
    classList: {
      toggle: (token, force) => {
        if (force) {
          classes.add(token);
        } else {
          classes.delete(token);
        }
      },
    },
    style: { colorScheme: "" },
    classes,
  };
}

describe("isTheme", () => {
  it("accepts the three themes", () => {
    expect(isTheme("light")).toBe(true);
    expect(isTheme("dark")).toBe(true);
    expect(isTheme("system")).toBe(true);
  });

  it("rejects anything else", () => {
    expect(isTheme(null)).toBe(false);
    expect(isTheme("blue")).toBe(false);
    expect(isTheme(undefined)).toBe(false);
  });
});

describe("readStoredTheme", () => {
  it("returns the default when storage is unavailable", () => {
    expect(readStoredTheme(null)).toBe(DEFAULT_THEME);
    expect(readStoredTheme(undefined)).toBe(DEFAULT_THEME);
  });

  it("reads a stored theme", () => {
    expect(readStoredTheme(fakeStorage("dark"))).toBe("dark");
    expect(readStoredTheme(fakeStorage("light"))).toBe("light");
  });

  it("falls back to the default for missing or invalid values", () => {
    expect(readStoredTheme(fakeStorage(null))).toBe(DEFAULT_THEME);
    expect(readStoredTheme(fakeStorage("nope"))).toBe(DEFAULT_THEME);
  });

  it("falls back when getItem throws", () => {
    const throwing = {
      getItem: () => {
        throw new Error("blocked");
      },
    };
    expect(readStoredTheme(throwing)).toBe(DEFAULT_THEME);
  });
});

describe("resolveTheme", () => {
  it("resolves system against the OS preference", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("keeps explicit themes", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});

describe("applyThemeClass", () => {
  it("adds the dark class and color-scheme for dark", () => {
    const root = fakeRoot();
    applyThemeClass(root, "dark");
    expect(root.classes.has("dark")).toBe(true);
    expect(root.style.colorScheme).toBe("dark");
  });

  it("removes the dark class for light", () => {
    const root = fakeRoot();
    root.classes.add("dark");
    applyThemeClass(root, "light");
    expect(root.classes.has("dark")).toBe(false);
    expect(root.style.colorScheme).toBe("light");
  });
});

describe("themeInitScript", () => {
  it("reads the shared storage key", () => {
    expect(themeInitScript).toContain(THEME_STORAGE_KEY);
  });
});
