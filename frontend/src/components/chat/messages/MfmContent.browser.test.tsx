import { render, screen } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { MfmContent } from "./MfmContent";
import "../../../styles.css";

it.each([160, 320])("keeps long URLs beside their icon and within a %ipx message", (width) => {
  const url = `https://example.com/${"verylong".repeat(30)}`;
  const view = render(
    <div
      style={{ width }}
      className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed"
    >
      <MfmContent content={url} />
    </div>,
  );
  const body = view.container.firstElementChild!;
  const link = screen.getByRole("link");
  expect(link).toHaveAttribute("href", url);
  const icon = link.querySelector("svg")!;
  const label = link.lastChild!;
  const text = label.nodeType === Node.TEXT_NODE ? label : label.firstChild!;
  const range = document.createRange();
  range.setStart(text, 0);
  range.setEnd(text, 1);
  const firstCharacter = range.getBoundingClientRect();
  const iconRect = icon.getBoundingClientRect();
  expect(firstCharacter.left).toBeGreaterThanOrEqual(iconRect.right);
  expect(Math.abs(firstCharacter.top - iconRect.top)).toBeLessThan(14);
  expect(link.getBoundingClientRect().height).toBeGreaterThan(firstCharacter.height * 2);
  expect(body.scrollWidth).toBeLessThanOrEqual(body.clientWidth);
});
