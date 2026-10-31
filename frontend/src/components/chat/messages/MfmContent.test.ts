import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import { MfmChannelsProvider } from "./MfmChannels";
import { parse } from "mfm-js";
import { MfmContent, type MfmContentProps } from "./MfmContent";

const render = (content: string, options: Omit<MfmContentProps, "content"> = {}) =>
  renderToStaticMarkup(createElement(MfmContent, { content, ...options }));

describe("MfmContent", () => {
  it("styles raw URLs as link chips while preserving destinations and custom labels", () => {
    const html = render("https://blog.example.com/a?x=1");
    expect(html).toContain('href="https://blog.example.com/a?x=1"');
    expect(html).toContain("bg-sky-100");
    expect(html).toContain('aria-hidden="true"');
    expect(html).toContain("blog.example.com/a?x=1</span></a>");
    expect(html).not.toContain(">https://");
    const labeled = render("[Read article](https://example.com/article)");
    expect(labeled).toContain(">Read article</a>");
    expect(labeled).not.toContain("bg-sky-100");
  });
  it("renders nested formatting and animated Unicode emoji", () => {
    const html = render("$[jelly.speed=2s **🎉**]");
    expect(html).toContain("mfm-effect mfm-jelly");
    expect(html).toContain("animation-duration:2s");
    expect(html).toContain("<strong>🎉</strong>");
  });
  it("accepts time units, negative delay and spin axes/direction", () => {
    const html = render("$[spin.x,left,speed=2500ms,delay=-1s 🐱]");
    expect(html).toContain("animation-duration:2.5s");
    expect(html).toContain("animation-delay:-1s");
    expect(html).toContain("animation-direction:reverse");
    expect(html).toContain("animation-name:mfm-spin-x");
    expect(render("$[spin.y,alternate 🐱]")).toContain("animation-name:mfm-spin-y");
    expect(render("$[spin.alternate 🐱]")).toContain("animation-direction:alternate");
  });
  it("bounds durations and ignores CSS-like arguments", () => {
    expect(render("$[bounce.speed=0.01s 🐱]")).toContain("animation-duration:0.5s");
    expect(render("$[spin.speed=999999s 🌟]")).toContain("animation-duration:86400s");
    expect(render("$[jelly.speed=evil 🎉]")).toContain("animation-duration:1s");
    expect(render("$[fg.color=red;position:fixed hello]")).not.toContain('style="position:fixed');
  });
  it.each([
    ["tada", "mfm-tada"],
    ["jelly", "mfm-jelly"],
    ["twitch", "mfm-twitch"],
    ["shake", "mfm-shake"],
    ["jump", "mfm-jump"],
    ["bounce", "mfm-bounce"],
    ["spin", "mfm-spin"],
    ["rainbow", "mfm-rainbow"],
    ["sparkle", "mfm-sparkles"],
    ["flip.h,v", "scale(-1, -1)"],
    ["flip.v", "scale(1, -1)"],
    ["x2", "font-size:2em"],
    ["x3", "font-size:3em"],
    ["x4", "font-size:4em"],
    ["scale.x=2,y=0.5", "scale(2, 0.5)"],
    ["position.x=-1,y=2", "translate(-1em, 2em)"],
    ["fg.color=123abc", "color:#123abc"],
    ["bg.color=1234", "background-color:#1234"],
    ["border.width=2,style=dotted,color=f00,radius=4", "border-style:dotted"],
    ["font.serif", "font-family:serif"],
    ["font.monospace", "font-family:monospace"],
    ["font.cursive", "font-family:cursive"],
    ["font.fantasy", "font-family:fantasy"],
    ["font.emoji", "font-family:emoji"],
    ["font.math", "font-family:math"],
    ["blur", "mfm-blur"],
    ["rotate.deg=-45", "rotate(-45deg)"],
    ["ruby", "<ruby>"],
    ["unixtime", 'class="mfm-unixtime"'],
    ["clickable.ev=hello", '<button type="button"'],
  ])("renders function %s", (fn, expected) => {
    const html = render(`$[${fn} ${fn === "unixtime" ? "0" : "hello"}]`);
    expect(html).toContain(expected);
    expect(html).not.toContain("$[");
  });
  it("bounds cumulative enlargement, transforms and borders", () => {
    expect(render("$[x4 $[x4 hi]]")).toContain("font-size:2em");
    expect(render("$[scale.x=999,y=-999 hi]")).toContain("scale(5, -5)");
    expect(render("$[position.x=999 hi]")).toContain("translate(2em, 0em)");
    const border = render("$[border.width=999,style=evil,radius=-2,noclip hi]");
    expect(border).toContain("border-width:100px");
    expect(border).toContain("border-style:solid");
    expect(border).toContain("border-radius:0");
    expect(border).not.toContain("overflow:clip");
  });
  it("renders ruby and preserves the absolute timestamp for local date display", () => {
    expect(render("$[ruby 漢字 かんじ]")).toContain("漢字<rp>(</rp><rt>かんじ</rt>");
    expect(render("$[ruby **漢字** かんじ]")).toContain(
      "<strong>漢字</strong><rp>(</rp><rt>かんじ</rt>",
    );
    expect(render("$[unixtime 0]")).toContain('dateTime="1970-01-01T00:00:00.000Z"');
    expect(render("$[unixtime invalid]")).toContain("$[unixtime invalid]");
  });
  it("preserves unknown functions, plain and code content", () => {
    expect(render("$[unknown 🐱] :unregistered_custom:")).toContain(
      "$[unknown 🐱] :unregistered_custom:",
    );
    expect(render("<plain>$[jelly 🎉] https://example.com</plain>")).toBe(
      "$[jelly 🎉] https://example.com",
    );
    expect(render("`$[jelly 🎉]`")).toContain("<code");
    expect(render("`$[jelly 🎉]`")).not.toContain("mfm-effect");
    expect(render("$[constructor hello]")).toBe("$[constructor hello]");
  });
  it("renders registered custom emoji and standard shortcode fallback", () => {
    expect(render(":smile:")).toBe("😄");
    const html = render(":party:", { emojiUrls: { party: "/emoji/party.webp" } });
    expect(html).toContain('src="/emoji/party.webp"');
    expect(html).toContain('alt=":party:"');
    expect(render(":party:", { emojiUrls: { party: "javascript:alert(1)" } })).not.toContain(
      "<img",
    );
  });
  it("renders every AST type in mfm-js with representative source text", () => {
    const fixtures: Record<string, string> = {
      text: "hello",
      unicodeEmoji: "🎉",
      emojiCode: ":smile:",
      bold: "**hi**",
      italic: "<i>hi</i>",
      strike: "~~hi~~",
      small: "<small>hi</small>",
      inlineCode: "`hi`",
      blockCode: "```js\nconst x = 1;\n```",
      plain: "<plain>**hi**</plain>",
      quote: "> hi",
      center: "<center>hi</center>",
      mathInline: "\\(x^2\\)",
      mathBlock: "\\[x^2\\]",
      url: "https://example.com",
      link: "[hi](https://example.com)",
      mention: "@alice",
      hashtag: "#tag",
      search: "猫 検索",
      fn: "$[jelly hi]",
    };
    for (const [type, source] of Object.entries(fixtures)) {
      expect(parse(source)[0]?.type).toBe(type);
      expect(render(source)).not.toBe("");
    }
    expect(render("\\(x^2\\)")).toContain('class="katex"');
    expect(render("\\[x^2\\]")).toContain("katex-display");
    expect(render("```js\nconst x = 1;\n```")).toContain("hljs-keyword");
    expect(render("```unknownlang\n<script>\n```")).toContain("&lt;script&gt;");
  });
  it("renders remote mentions, hashtags and search without automatically fetching external services", () => {
    expect(render("@alice@example.com")).toContain('href="https://example.com/@alice"');
    expect(render("#猫")).toBe("#猫");
    expect(render("猫 検索")).toContain("https://www.google.com/search?q=%E7%8C%AB");
  });
  it("links only exact existing channel names, including inside decorations", () => {
    const channels = [
      { id: "channel-1", channel_name: "開発", deleted_at: null },
      { id: "channel-2", channel_name: "deleted", deleted_at: "2026-10-25" },
    ];
    const html = renderToStaticMarkup(
      createElement(MfmChannelsProvider, {
        channels,
        children: createElement(MfmContent, { content: "$[jelly #開発] #未登録 #deleted #開発中" }),
      }),
    );
    expect(html).toContain('href="/channels/channel-1"');
    expect(html.match(/<a /g)).toHaveLength(1);
    expect(html).toContain("#未登録 #deleted #開発中");
    expect(html).not.toContain("google.com");
    expect(html).not.toContain('target="_blank"');
  });
  it("escapes HTML, denies unsafe links and KaTeX trusted commands", () => {
    const html = render("<script>alert(1)</script> [click](javascript:alert(1))");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain('href="javascript:');
    expect(render("[docs](https://example.com)")).toContain('rel="noopener noreferrer"');
    expect(render("[https://inner.example](https://example.com)").match(/<a /g)).toHaveLength(1);
    expect(render("\\(\\href{javascript:alert(1)}{x}\\)")).not.toContain('href="javascript:');
    expect(render("\\(\\htmlStyle{position:fixed}{x}\\)")).not.toContain('style="position:fixed');
  });
});

it("bounds mixed nested transforms and font enlargement, including axis swaps and negatives", () => {
  const html = render(
    "$[scale.x=5,y=-5 $[rotate.deg=90 $[scale.x=5,y=5 $[x4 $[scale.x=5,y=5 ■]]]]]",
  );
  const scales = [...html.matchAll(/transform:scale\(([-\d.]+), ([-\d.]+)\)/g)];
  const fonts = [...html.matchAll(/font-size:([\d.]+)em/g)];
  const total =
    scales.reduce(
      (value, match) => value * Math.max(Math.abs(Number(match[1])), Math.abs(Number(match[2]))),
      1,
    ) * fonts.reduce((value, match) => value * Number(match[1]), 1);
  expect(scales).toHaveLength(3);
  expect(total).toBeLessThanOrEqual(8);
  expect(html).toContain("scale(5, -5)");
  expect(render("$[position.x=50,y=-50 hi]")).toContain("translate(2em, -2em)");
});
