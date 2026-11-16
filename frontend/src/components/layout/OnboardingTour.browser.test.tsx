import "../../styles.css";
import { page } from "vite-plus/test/browser/context";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { OnboardingProvider, OnboardingButton } from "./OnboardingTour";

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("zoo-locale", "ja");
});
function tour(userId: string | null) {
  return (
    <LocaleProvider>
      <OnboardingProvider key={userId} userId={userId}>
        <OnboardingButton />
      </OnboardingProvider>
    </LocaleProvider>
  );
}
it("persists skipping, reopens at the beginning, and isolates accounts", async () => {
  const view = render(tour("alice"));
  await userEvent.click(await screen.findByRole("button", { name: "次へ" }));
  await userEvent.click(screen.getByRole("button", { name: "スキップ" }));
  await waitFor(() =>
    expect(screen.queryByRole("complementary", { name: "使い方ツアー" })).toBeNull(),
  );
  expect(localStorage.getItem("zoo:onboarding:v1:alice")).toBe("done");
  view.unmount();
  const returning = render(tour("alice"));
  expect(screen.queryByRole("complementary", { name: "使い方ツアー" })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "初期ツアーを開く" }));
  expect(await screen.findByRole("heading", { name: "Zooへようこそ" })).toBeVisible();
  returning.rerender(tour("bob"));
  expect(await screen.findByRole("complementary", { name: "使い方ツアー" })).toBeVisible();
  expect(localStorage.getItem("zoo:onboarding:v1:bob")).toBeNull();
});
it("can dismiss with Escape even when storage is blocked", async () => {
  const getItem = localStorage.getItem.bind(localStorage);
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(function (key: string) {
    if (key.startsWith("zoo:onboarding:")) throw new Error("blocked");
    return getItem(key);
  });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  try {
    render(tour("alice"));
    await screen.findByRole("complementary", { name: "使い方ツアー" });
    await userEvent.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("complementary", { name: "使い方ツアー" })).toBeNull(),
    );
  } finally {
    vi.restoreAllMocks();
  }
});
it("does not open automatically before the account is known", () => {
  render(tour(null));
  expect(screen.queryByRole("complementary", { name: "使い方ツアー" })).toBeNull();
});

it("moves through real destinations, including Slack, and preserves the tour", async () => {
  const navigate = vi.fn();
  const view = render(
    <LocaleProvider>
      <OnboardingProvider userId="alice" onStepChange={navigate} channelSelected={false}>
        <OnboardingButton />
      </OnboardingProvider>
    </LocaleProvider>,
  );
  await userEvent.click(await screen.findByRole("button", { name: "次へ" }));
  expect(navigate).toHaveBeenLastCalledWith("channels");
  expect(screen.getByRole("button", { name: "次へ" })).toBeDisabled();
  view.rerender(
    <LocaleProvider>
      <OnboardingProvider userId="alice" onStepChange={navigate} channelSelected>
        <OnboardingButton />
      </OnboardingProvider>
    </LocaleProvider>,
  );
  for (let i = 0; i < 3; i += 1)
    await userEvent.click(screen.getByRole("button", { name: "次へ" }));
  expect(navigate.mock.calls.map(([step]) => step)).toEqual([
    "channels",
    "messages",
    "notifications",
    "slack",
  ]);
  expect(await screen.findByRole("heading", { name: "Slackと連携する" })).toBeVisible();
  expect(localStorage.getItem("zoo:onboarding:v1:alice")).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "戻る" }));
  expect(navigate).toHaveBeenLastCalledWith("notifications");
});
it("keeps the current step when navigation fails and allows retrying", async () => {
  const navigate = vi.fn().mockRejectedValueOnce(new Error("failed")).mockResolvedValue(undefined);
  render(
    <LocaleProvider>
      <OnboardingProvider userId="alice" onStepChange={navigate}>
        screen
      </OnboardingProvider>
    </LocaleProvider>,
  );
  await userEvent.click(await screen.findByRole("button", { name: "次へ" }));
  expect(await screen.findByRole("alert")).toBeVisible();
  expect(screen.getByRole("heading", { name: "Zooへようこそ" })).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "次へ" }));
  expect(await screen.findByRole("heading", { name: "チャンネルを探す" })).toBeVisible();
});
it("highlights the actual target while keeping it interactive and cleans up", async () => {
  const action = vi.fn();
  render(
    <LocaleProvider>
      <OnboardingProvider userId="alice">
        <button data-onboarding="channels" onClick={action}>
          Channel list
        </button>
      </OnboardingProvider>
    </LocaleProvider>,
  );
  await userEvent.click(await screen.findByRole("button", { name: "次へ" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Channel list" })).toHaveClass("onboarding-target"),
  );
  await userEvent.click(screen.getByRole("button", { name: "Channel list" }));
  expect(action).toHaveBeenCalledOnce();
  await userEvent.click(screen.getByRole("button", { name: "スキップ" }));
  await waitFor(() => expect(document.documentElement.dataset.onboardingActive).toBeUndefined());
  expect(screen.getByRole("button", { name: "Channel list" })).not.toHaveClass("onboarding-target");
});

it("reserves space for the real screen beside or above the tour", async () => {
  for (const width of [375, 1280]) {
    await page.viewport(width, 800);
    const view = render(
      <LocaleProvider>
        <OnboardingProvider userId="layout">
          <div data-app-shell className="fixed inset-x-0 top-0 h-dvh w-full">
            Actual screen
          </div>
        </OnboardingProvider>
      </LocaleProvider>,
    );
    await waitFor(() => {
      const screenBox = document.querySelector("[data-app-shell]")!.getBoundingClientRect();
      const panelBox = document.querySelector("[data-onboarding-panel]")!.getBoundingClientRect();
      if (width < 768) expect(screenBox.bottom).toBeLessThanOrEqual(panelBox.top);
      else expect(screenBox.right).toBeLessThanOrEqual(panelBox.left);
    });
    view.unmount();
  }
});
