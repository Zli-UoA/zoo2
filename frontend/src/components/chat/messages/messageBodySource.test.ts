import { describe, expect, it } from "vite-plus/test";

import { IS_COMPONENTS_V2 } from "../components/parse";
import { messageBodySource } from "./messageBodySource";

const textDisplay = { type: 10, content: "部品の本文" };
const card = JSON.stringify({
  title: "デプロイ完了",
  description: "本番に反映しました",
  url: "https://example.com/deploys/1",
  source: "CI",
  context: "",
  author: "",
  badge: "",
});
const cardBlocks = JSON.stringify([{ type: "card", card: JSON.parse(card) }]);
const buttonBlocks = JSON.stringify([{ type: "button", action_id: "go", label: "開く" }]);
const hiddenComponents = JSON.stringify({ flags: IS_COMPONENTS_V2, components: [textDisplay] });
const visibleComponents = JSON.stringify({ flags: 0, components: [textDisplay] });

describe("messageBodySource", () => {
  it("何もなければ、本文（content）を描く", () => {
    expect(messageBodySource({})).toEqual({ kind: "text" });
    expect(messageBodySource({ components: null, appBlocks: null, card: null })).toEqual({
      kind: "text",
    });
  });

  it("本文を隠すフラグのある部品は、本文の代わりに部品を描く", () => {
    expect(messageBodySource({ components: hiddenComponents })).toEqual({ kind: "hidden" });
  });

  it("本文を隠さない部品は、本文も描く", () => {
    expect(messageBodySource({ components: visibleComponents })).toEqual({ kind: "text" });
  });

  it("読めない部品は、ないものとして扱う", () => {
    expect(messageBodySource({ components: "{}" })).toEqual({ kind: "text" });
    expect(messageBodySource({ components: "not json" })).toEqual({ kind: "text" });
  });

  it("App のカードのブロックは、本文の代わりにブロックを描く", () => {
    expect(messageBodySource({ appBlocks: cardBlocks })).toEqual({ kind: "hidden" });
  });

  it("カードのないブロック（ボタンなど）は、本文も描く", () => {
    expect(messageBodySource({ appBlocks: buttonBlocks })).toEqual({ kind: "text" });
  });

  it("連携カードは、本文の代わりにカードを描く", () => {
    const source = messageBodySource({ card });
    expect(source.kind).toBe("cards");
    expect(source.kind === "cards" && source.cards[0]?.title).toBe("デプロイ完了");
  });

  it("読めないカードは、ないものとして扱う", () => {
    expect(messageBodySource({ card: "not json" })).toEqual({ kind: "text" });
    expect(messageBodySource({ card: JSON.stringify({ cards: [] }) })).toEqual({ kind: "text" });
  });

  it("部品があるときは、連携カードを見ない（部品を優先する）", () => {
    expect(messageBodySource({ components: visibleComponents, card })).toEqual({ kind: "text" });
    expect(messageBodySource({ components: hiddenComponents, card })).toEqual({ kind: "hidden" });
  });
});
