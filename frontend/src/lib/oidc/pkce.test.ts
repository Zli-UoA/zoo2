import { describe, expect, it } from "vite-plus/test";

import {
  base64UrlEncode,
  createChallenge,
  createNonce,
  createPkcePair,
  createState,
  createVerifier,
  isUnreserved,
} from "./pkce";

describe("base64UrlEncode", () => {
  it("encodes without padding and with URL-safe characters", () => {
    expect(base64UrlEncode(new Uint8Array([0xfb, 0xff, 0x3e]))).toBe("-_8-");
    expect(base64UrlEncode(new Uint8Array([]))).toBe("");
  });
});

describe("createChallenge", () => {
  it("matches the RFC 7636 Appendix B S256 vector", async () => {
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    await expect(createChallenge(verifier)).resolves.toBe(
      "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
    );
  });

  it("is deterministic for the same verifier", async () => {
    const verifier = createVerifier();
    expect(await createChallenge(verifier)).toBe(await createChallenge(verifier));
  });
});

describe("createVerifier", () => {
  it("is 43 URL-safe characters and unique per call", () => {
    const first = createVerifier();
    const second = createVerifier();
    expect(first).toHaveLength(43);
    expect(isUnreserved(first)).toBe(true);
    expect(first).not.toBe(second);
  });
});

describe("createState / createNonce", () => {
  it("produces unique URL-safe values", () => {
    const state = createState();
    const nonce = createNonce();
    expect(state).not.toBe(createState());
    expect(nonce).not.toBe(createNonce());
    expect(isUnreserved(state)).toBe(true);
    expect(isUnreserved(nonce)).toBe(true);
  });
});

describe("createPkcePair", () => {
  it("returns a verifier and its S256 challenge", async () => {
    const pair = await createPkcePair();
    expect(pair.method).toBe("S256");
    expect(pair.challenge).toBe(await createChallenge(pair.verifier));
  });
});
