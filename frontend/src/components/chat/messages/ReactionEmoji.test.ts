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

    expect(html).toContain('style="width:20px;height:20px"');
    expect(html).toContain("😀");
    expect(html).not.toContain("<img");
  });

  it("renders the image in the same sized slot after the catalog loads", () => {
    const html = renderToStaticMarkup(
      createElement(ReactionEmoji, { emoji: "kitchen:1f600_1f431", catalog }),
    );

    expect(html).toContain('style="width:20px;height:20px"');
    expect(html).toContain("<img");
    expect(html).toContain('alt=""');
    expect(html).toContain('width="20"');
  });

  it("renders a plain emoji in the same slot as a kitchen emoji", () => {
    const html = renderToStaticMarkup(createElement(ReactionEmoji, { emoji: "👍", catalog: null }));

    expect(html).toContain("width:20px;height:20px;font-size:20px");
    expect(html).toContain("👍</span>");
  });
});
