import { expect, it, vi } from "vite-plus/test";
import { readStoredSession, writeStoredSession } from "./stored-session";

it.each([
  "{invalid",
  "null",
  JSON.stringify({ binding: "expected", tokens: { accessToken: "token" } }),
  JSON.stringify({
    binding: "expected",
    tokens: { accessToken: "token", refreshToken: 42, idToken: null, expiresAt: 5000 },
  }),
])("discards malformed saved session: %s", (raw) => {
  const storage = {
    getItem: (key: string) => (key === "zoo.oidc.session" ? raw : null),
    setItem: vi.fn(),
    removeItem: vi.fn(),
  };
  expect(readStoredSession(storage, "expected", 1000)).toBeNull();
  expect(storage.removeItem).toHaveBeenCalledWith("zoo.oidc.session");
});

it("continues without persistence when sessionStorage is unavailable", () => {
  const storage = {
    getItem: () => {
      throw new Error("Storage disabled");
    },
    setItem: () => {
      throw new Error("Storage disabled");
    },
    removeItem: () => {
      throw new Error("Storage disabled");
    },
  };
  expect(readStoredSession(storage, "expected", 1000)).toBeNull();
  expect(() =>
    writeStoredSession(storage, "expected", {
      accessToken: "token",
      refreshToken: null,
      idToken: null,
      expiresAt: 5000,
    }),
  ).not.toThrow();
});
