import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import { MessageCard, parseMessageCard } from "./MessageCard";

describe("Message card format", () => {
  it("accepts a complete card and preserves Unicode", () => {
    const card = {
      title: "PR #42 · 修正",
      url: "https://github.com/a/b/pull/42",
      description: "本文",
      source: "GitHub",
      context: "a/b",
      author: "octocat",
      badge: "pull_request",
    };
    expect(parseMessageCard(JSON.stringify(card))).toEqual(card);
  });
  it("falls back to ordinary text for invalid or incomplete cards", () => {
    for (const value of ["hello", "{", "null", '{"title":42}']) {
      expect(parseMessageCard(value)).toBeNull();
    }
  });
});

describe("Message card rendering", () => {
  it("escapes external text and excludes unsafe links", () => {
    const card = {
      title: "<script>alert(1)</script>",
      url: "javascript:alert(1)",
      description: "<img onerror=alert(1)>",
      source: "GitHub",
      context: "a/b",
      author: "octocat",
      badge: "push",
    };
    const html = renderToStaticMarkup(createElement(MessageCard, { card }));
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<script>");
    const safe = renderToStaticMarkup(
      createElement(MessageCard, { card: { ...card, url: "https://github.com/a/b" } }),
    );
    expect(safe).toContain('href="https://github.com/a/b"');
    expect(safe).toContain('rel="noopener noreferrer"');
  });
});
