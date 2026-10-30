import { describe, expect, it } from "vite-plus/test";
import { previewUrls, proxyImageUrl } from "./linkPreviews";

describe("link previews", () => {
  it("extracts visible links, deduplicates and limits cards", () => {
    expect(
      previewUrls("https://example.com。 [site](https://other.example/p) https://example.com"),
    ).toEqual(["https://example.com/", "https://other.example/p"]);
    expect(
      previewUrls("https://a.example https://b.example https://c.example https://d.example"),
    ).toHaveLength(3);
  });
  it("ignores code, plain text, silent links and non-HTTP URLs", () => {
    expect(
      previewUrls(
        "`https://code.example`\n```\nhttps://block.example\n```\n<plain>https://plain.example</plain> ?[silent](https://silent.example) [bad](javascript:alert)",
      ),
    ).toEqual([]);
  });
  it("sends images only to the configured proxy with encoded source and thumbnail flags", () => {
    const image = "https://images.example/a.png?x=1&y=2";
    const proxy = new URL(proxyImageUrl("https://media.example/", image)!);
    expect(proxy.origin).toBe("https://media.example");
    expect(proxy.searchParams.get("url")).toBe(image);
    expect(proxy.searchParams.get("preview")).toBe("1");
    expect(proxy.searchParams.get("static")).toBe("1");
    expect(proxyImageUrl("/media/", image)).toMatch(/^\/media\/\?url=/);
    expect(proxyImageUrl("", image)).toBeNull();
    expect(proxyImageUrl("//evil.example", image)).toBeNull();
    expect(proxyImageUrl("https://media.example", "javascript:alert(1)")).toBeNull();
    expect(proxyImageUrl("https://user:pass@media.example", image)).toBeNull();
  });
});
