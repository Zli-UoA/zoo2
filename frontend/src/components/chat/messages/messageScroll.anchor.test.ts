// @vitest-environment happy-dom
import { describe, expect, it } from "vite-plus/test";
import { findVisibleMessageAnchor, getAnchorScrollDelta } from "./messageScroll";

function rect(top: number, bottom: number): DOMRect {
  return {
    top,
    bottom,
    left: 0,
    right: 0,
    width: 0,
    height: bottom - top,
    x: 0,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

function row(id: string, top: number, bottom: number): HTMLElement {
  const element = document.createElement("div");
  element.dataset.messageId = id;
  element.getBoundingClientRect = () => rect(top, bottom);
  return element;
}

describe("findVisibleMessageAnchor", () => {
  it("ビューポート内にある最初の行を、上端からの距離つきで返す", () => {
    const root = document.createElement("div");
    root.getBoundingClientRect = () => rect(100, 300);
    root.append(row("above", 0, 100), row("visible", 120, 160), row("below", 320, 360));

    expect(findVisibleMessageAnchor(root)).toEqual({ id: "visible", offset: 20 });
  });

  it("見える行がなければ、直前のアンカーを上書きしないため null を返す", () => {
    const root = document.createElement("div");
    root.getBoundingClientRect = () => rect(100, 300);
    root.append(row("above", 0, 50));

    expect(findVisibleMessageAnchor(root)).toBeNull();
  });

  it("ビューポートの境界に接する行は表示中とみなさない", () => {
    const root = document.createElement("div");
    root.getBoundingClientRect = () => rect(100, 300);
    root.append(row("edge", 50, 100));

    expect(findVisibleMessageAnchor(root)).toBeNull();
  });
});

it("getAnchorScrollDelta は記録位置への移動量を返す", () => {
  expect(getAnchorScrollDelta(180, 100, 20)).toBe(60);
  expect(getAnchorScrollDelta(120, 100, 20)).toBe(0);
  expect(getAnchorScrollDelta(80, 100, 20)).toBe(-40);
});
