import { describe, expect, it } from "vite-plus/test";
import { parsePdfPageNumber } from "./pdf-preview";

describe("parsePdfPageNumber", () => {
  it("parses ASCII and full-width digits and clamps within range", () => {
    expect(parsePdfPageNumber("1", 10)).toBe(1);
    expect(parsePdfPageNumber(" 7 ", 10)).toBe(7);
    expect(parsePdfPageNumber("１０", 12)).toBe(10);
    expect(parsePdfPageNumber("0", 10)).toBe(1);
    expect(parsePdfPageNumber("999", 10)).toBe(10);
  });

  it("rejects empty, non-numeric, or negative strings", () => {
    expect(parsePdfPageNumber("", 10)).toBeNull();
    expect(parsePdfPageNumber("   ", 10)).toBeNull();
    expect(parsePdfPageNumber("abc", 10)).toBeNull();
    expect(parsePdfPageNumber("-2", 10)).toBeNull();
    expect(parsePdfPageNumber("1.5", 10)).toBeNull();
    expect(parsePdfPageNumber("3", 0)).toBeNull();
  });
});
