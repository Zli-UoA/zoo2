import { LocaleProvider } from "../../i18n/LocaleProvider";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import { MessageCard, parseMessageCard, parseMessageCards, type CardData } from "./MessageCard";

const renderCard = (card: CardData, attachment = false) =>
  renderToStaticMarkup(
    createElement(LocaleProvider, { children: createElement(MessageCard, { card, attachment }) }),
  );

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
  it("renders attachment fields, a code commit link and a footer without unsafe URLs", () => {
    const card = parseMessageCard(
      JSON.stringify({
        title: "Deployment to production",
        color: "#2da44e",
        source: "Zli-UoA/zoo2",
        fields: [
          { title: "Status", value: "Completed", short: true },
          {
            title: "Commit",
            value: "103eb20 (main)",
            short: true,
            code: true,
            url: "https://github.com/Zli-UoA/zoo2/commit/103eb20",
          },
          { title: "Workflow", value: "<img onerror=alert(1)>", url: "javascript:alert(1)" },
        ],
      }),
    )!;
    const html = renderCard(card, true);
    expect(html).toContain("sm:grid-cols-2");
    expect(html).toContain("<code");
    expect(html).toContain("Zli-UoA/zoo2");
    expect(html).toContain('href="https://github.com/Zli-UoA/zoo2/commit/103eb20"');
    expect(html).not.toContain("javascript:");
    expect(html).toContain("&lt;img");
    expect(parseMessageCard('{"title":"bad","fields":[{"title":"x","value":4}]}')).toBeNull();
  });
  it("renders all Slack attachments and only permits supported colors", () => {
    const cards = parseMessageCards(
      JSON.stringify({
        cards: [
          { title: "Ready", color: "good" },
          { title: "Failed", color: "#ff0000" },
          { title: "Untrusted", color: "red; background: url(evil)" },
        ],
      }),
    );
    expect(cards).toHaveLength(3);
    const html = cards!.map((card) => renderCard(card)).join("");
    expect(html).toContain("border-left-color:#2eb886");
    expect(html).toContain("border-left-color:#ff0000");
    expect(html).not.toContain("evil");
    expect(parseMessageCards('{"cards":[]}')).toBeNull();
    expect(parseMessageCards('{"cards":[{"title":"ok"},null]}')).toBeNull();
  });
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
    const html = renderCard(card);
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<script>");
    const safe = renderCard({ ...card, url: "https://github.com/a/b" });
    expect(safe).toContain('href="https://github.com/a/b"');
    expect(safe).toContain('rel="noopener noreferrer"');
  });
});
