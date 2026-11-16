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
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(localStorage.getItem("zoo:onboarding:v1:alice")).toBe("done");
  view.unmount();
  const returning = render(tour("alice"));
  expect(screen.queryByRole("dialog")).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "初期ツアーを開く" }));
  expect(await screen.findByRole("heading", { name: "Zooへようこそ" })).toBeVisible();
  returning.rerender(tour("bob"));
  expect(await screen.findByRole("dialog")).toBeVisible();
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
    await screen.findByRole("dialog");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  } finally {
    vi.restoreAllMocks();
  }
});
it("does not open automatically before the account is known", () => {
  render(tour(null));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("offers Slack setup without starting OAuth during the tour", async () => {
  const openSlack = vi.fn();
  render(
    <LocaleProvider>
      <OnboardingProvider userId="alice" onOpenSlack={openSlack}>
        <OnboardingButton />
      </OnboardingProvider>
    </LocaleProvider>,
  );
  await screen.findByRole("dialog");
  for (let i = 0; i < 4; i += 1)
    await userEvent.click(screen.getByRole("button", { name: "次へ" }));
  expect(screen.getByRole("heading", { name: "Slackと連携する" })).toBeVisible();
  expect(openSlack).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Slack連携を設定" }));
  expect(openSlack).toHaveBeenCalledOnce();
  expect(localStorage.getItem("zoo:onboarding:v1:alice")).toBe("done");
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
