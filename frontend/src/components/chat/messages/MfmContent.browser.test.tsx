import { render, screen } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { MfmContent } from "./MfmContent";
import "../../../styles.css";

it.each([160, 320])("wraps long inline URLs within a %ipx message without padding", (width) => {
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
  expect(getComputedStyle(link).paddingTop).toBe("0px");
  expect(getComputedStyle(link).paddingBottom).toBe("0px");
  expect(getComputedStyle(link).cursor).toBe("pointer");
  expect(link.getClientRects().length).toBeGreaterThan(1);
  expect(body.scrollWidth).toBeLessThanOrEqual(body.clientWidth);
});

it.each([160, 640])("keeps heading size, wrapping and spacing within a %ipx message", (width) => {
  const view = render(
    <div
      style={{ width }}
      className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed"
    >
      <MfmContent content={`# ${"LongHeading".repeat(12)}\n## Smaller\nBody`} />
    </div>,
  );
  const [large, small] = screen.getAllByRole("heading");
  expect(parseFloat(getComputedStyle(large).fontSize)).toBeGreaterThan(
    parseFloat(getComputedStyle(small).fontSize),
  );
  expect(getComputedStyle(large).marginTop).toBe("0px");
  const body = view.container.firstElementChild!;
  expect(body.scrollWidth).toBeLessThanOrEqual(body.clientWidth);
});
