import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { ReactionEmoji } from "./ReactionEmoji";

const catalog: KitchenCatalog = {
  emojis: [
    ["1f600", "😀", "grinning"],
    ["1f431", "🐱", "cat"],
  ],
  dates: ["20230301"],
  pairs: { "0_1": 0 },
};

describe("ReactionEmoji", () => {
  it("keeps the image slot before the kitchen catalog loads", () => {
    const html = renderToStaticMarkup(
      createElement(ReactionEmoji, { emoji: "kitchen:1f600_1f431", catalog: null }),
    );

    expect(html).toContain('style="width:28px;height:28px"');
    expect(html).toContain("😀");
    expect(html).not.toContain("<img");
  });

  it("renders the image in the same sized slot after the catalog loads", () => {
    const html = renderToStaticMarkup(
      createElement(ReactionEmoji, { emoji: "kitchen:1f600_1f431", catalog }),
    );

    expect(html).toContain('style="width:28px;height:28px"');
    expect(html).toContain("<img");
    expect(html).toContain('alt=""');
    expect(html).toContain('width="28"');
  });

  it("renders a plain emoji without the fixed kitchen slot", () => {
    const html = renderToStaticMarkup(createElement(ReactionEmoji, { emoji: "👍", catalog: null }));

    expect(html).toBe("<span>👍</span>");
  });
});
