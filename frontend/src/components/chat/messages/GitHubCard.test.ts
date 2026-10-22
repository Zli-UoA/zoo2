import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import { GITHUB_CARD_PREFIX, GitHubCard, parseGitHubCard } from "./GitHubCard";

describe("GitHub card format", () => {
  it("accepts a complete card and preserves Unicode", () => {
    const card = {
      title: "PR #42 · 修正",
      url: "https://github.com/a/b/pull/42",
      description: "本文",
      repository: "a/b",
      actor: "octocat",
      event: "pull_request",
    };
    expect(parseGitHubCard(GITHUB_CARD_PREFIX + JSON.stringify(card))).toEqual(card);
  });
  it("falls back to ordinary text for invalid or incomplete cards", () => {
    for (const value of [
      "hello",
      GITHUB_CARD_PREFIX + "{",
      GITHUB_CARD_PREFIX + "null",
      GITHUB_CARD_PREFIX + '{"title":42}',
    ]) {
      expect(parseGitHubCard(value)).toBeNull();
    }
  });
});

describe("GitHub card rendering", () => {
  it("escapes external text and excludes unsafe links", () => {
    const card = {
      title: "<script>alert(1)</script>",
      url: "javascript:alert(1)",
      description: "<img onerror=alert(1)>",
      repository: "a/b",
      actor: "octocat",
      event: "push",
    };
    const html = renderToStaticMarkup(createElement(GitHubCard, { card }));
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<script>");
    const safe = renderToStaticMarkup(
      createElement(GitHubCard, { card: { ...card, url: "https://github.com/a/b" } }),
    );
    expect(safe).toContain('href="https://github.com/a/b"');
    expect(safe).toContain('rel="noopener noreferrer"');
  });
});
