import { describe, expect, it } from "vite-plus/test";

import { audienceIncludes, decodeJwtPayload, issuerMatches } from "./jwt";
import { base64UrlEncode } from "./pkce";

function makeJwt(payload: unknown, header: unknown = { alg: "ES256", typ: "JWT" }): string {
  const encode = (value: unknown) =>
    base64UrlEncode(new TextEncoder().encode(JSON.stringify(value)));
  return `${encode(header)}.${encode(payload)}.signature`;
}

describe("decodeJwtPayload", () => {
  it("decodes a UTF-8 payload", () => {
    const token = makeJwt({ sub: "abc", name: "山田 太郎", exp: 1700000000 });
    expect(decodeJwtPayload(token)).toEqual({ sub: "abc", name: "山田 太郎", exp: 1700000000 });
  });

  it("returns null for malformed tokens", () => {
    expect(decodeJwtPayload("not-a-jwt")).toBeNull();
    expect(decodeJwtPayload("a.b")).toBeNull();
    expect(decodeJwtPayload("a.!!!.c")).toBeNull();
    expect(decodeJwtPayload(`${makeJwt([1, 2])}`)).toBeNull();
  });
});

describe("audienceIncludes", () => {
  it("accepts a string or array audience", () => {
    expect(audienceIncludes("zoo", "zoo")).toBe(true);
    expect(audienceIncludes(["other", "zoo"], "zoo")).toBe(true);
    expect(audienceIncludes("other", "zoo")).toBe(false);
    expect(audienceIncludes(undefined, "zoo")).toBe(false);
  });
});

describe("issuerMatches", () => {
  it("ignores a trailing slash difference", () => {
    expect(
      issuerMatches("https://id.example/oauth2/openid/zoo", "https://id.example/oauth2/openid/zoo"),
    ).toBe(true);
    expect(
      issuerMatches(
        "https://id.example/oauth2/openid/zoo/",
        "https://id.example/oauth2/openid/zoo",
      ),
    ).toBe(true);
    expect(issuerMatches("https://evil.example", "https://id.example/oauth2/openid/zoo")).toBe(
      false,
    );
    expect(issuerMatches(undefined, "https://id.example")).toBe(false);
  });
});
