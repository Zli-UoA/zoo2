// @vitest-environment happy-dom
import { createElement } from "react";
import { BookOpenCheckIcon } from "lucide-react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vite-plus/test";

import { ScreenHeader } from "./PaneHeader";

afterEach(cleanup);

const baseProps = {
  icon: BookOpenCheckIcon,
  titleId: "test-title",
  title: "タイトル",
};

it("does not make the heading focusable when titleRef is omitted", () => {
  render(createElement(ScreenHeader, baseProps));
  expect(screen.getByRole("heading").hasAttribute("tabindex")).toBe(false);
});

it("attaches titleRef to the heading and makes it focusable", () => {
  const titleRef: { current: HTMLHeadingElement | null } = { current: null };
  render(
    createElement(ScreenHeader, {
      ...baseProps,
      titleRef: (node: HTMLHeadingElement | null) => {
        titleRef.current = node;
      },
    }),
  );
  expect(titleRef.current).toBe(screen.getByRole("heading"));
  expect(titleRef.current?.getAttribute("tabindex")).toBe("-1");
});
