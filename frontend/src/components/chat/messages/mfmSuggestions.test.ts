import { describe, expect, it } from "vite-plus/test";
import { insertMfm, mfmOptions, mfmQuery } from "./mfmSuggestions";

describe("MFM suggestions", () => {
  it.each(["$", "$[", "$je", "$[je"])("recognizes %s after text at the caret", (token) => {
    const value = `hello ${token} tail`;
    expect(mfmQuery(value, 6 + token.length)).toEqual({
      start: 6,
      end: 6 + token.length,
      query: token.replaceAll("$", "").replaceAll("[", ""),
    });
  });
  it("filters by function prefix", () => {
    expect(mfmOptions("je")).toEqual(["jelly"]);
    expect(mfmOptions("SP")).toEqual(["spin", "sparkle"]);
    expect(mfmOptions("missing")).toEqual([]);
  });
  it.each(["\\$", "`$", "```js\n$", "$[jelly ", "$10", "$[jelly]", "https://example.com/$"])(
    "does not offer functions inside escapes, code, arguments or an unmatched prefix: %s",
    (value) => {
      const query = mfmQuery(value, value.length);
      // A numeric prefix matches no functions, so no popup is shown.
      expect(query ? mfmOptions(query.query) : []).toEqual([]);
    },
  );
  it("inserts a complete function preserving text after the caret", () => {
    const value = "hello $je tail";
    expect(insertMfm(value, mfmQuery(value, 9)!, "jelly")).toEqual({
      value: "hello $[jelly ] tail",
      caret: 14,
    });
    expect(insertMfm("$[fo", mfmQuery("$[fo", 4)!, "font")).toEqual({
      value: "$[font.serif ]",
      caret: 13,
    });
  });
});
