import { render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";

import { AppBadgeBridge } from "./AppBadgeBridge";

const state = vi.hoisted(() => ({ unreadCount: 2 }));
const setAppBadge = vi.hoisted(() => vi.fn());

vi.mock("../../hooks/useUnifiedNotifications", () => ({
  useUnifiedNotifications: () => ({ unreadCount: state.unreadCount }),
}));
vi.mock("../../lib/desktop", () => ({ isDesktopSession: () => false }));
vi.mock("../../lib/pwa/badge", () => ({ setAppBadge }));

afterEach(() => {
  setAppBadge.mockClear();
  state.unreadCount = 2;
});

it("reflects the unread count and follows changes", () => {
  const view = render(<AppBadgeBridge />);
  expect(setAppBadge).toHaveBeenLastCalledWith(2);
  state.unreadCount = 0;
  view.rerender(<AppBadgeBridge />);
  expect(setAppBadge).toHaveBeenLastCalledWith(0);
});

it("resyncs when the page becomes visible again", () => {
  render(<AppBadgeBridge />);
  setAppBadge.mockClear();
  document.dispatchEvent(new Event("visibilitychange"));
  expect(setAppBadge).toHaveBeenCalledWith(2);
});

it("does not clear the shared badge when it unmounts", () => {
  const view = render(<AppBadgeBridge />);
  setAppBadge.mockClear();
  view.unmount();
  expect(setAppBadge).not.toHaveBeenCalled();
});
