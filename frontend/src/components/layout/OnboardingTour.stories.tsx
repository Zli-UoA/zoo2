import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, fn } from "storybook/test";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { OnboardingProvider, OnboardingButton } from "./OnboardingTour";

const meta = {
  title: "Features/Onboarding/Tour",
  component: OnboardingProvider,
  parameters: { a11y: { test: "error" } },
  args: { userId: "onboarding-story", children: <OnboardingButton />, onOpenSlack: fn() },
  beforeEach: () => {
    localStorage.removeItem("zoo:onboarding:v1:onboarding-story");
  },
} satisfies Meta<typeof OnboardingProvider>;
export default meta;
type Story = StoryObj<typeof meta>;

export const FirstVisit: Story = {};
export const CompleteAndReplay: Story = {
  play: playForImplementation(async () => {
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("button", { name: "次へ" }));
    await expect(screen.getByRole("heading", { name: "チャンネルを探す" })).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "戻る" }));
    await expect(screen.getByRole("heading", { name: "Zooへようこそ" })).toBeVisible();
    for (let i = 0; i < 5; i += 1)
      await userEvent.click(screen.getByRole("button", { name: "次へ" }));
    await userEvent.click(screen.getByRole("button", { name: "Zooを使い始める" }));
    await expect(localStorage.getItem("zoo:onboarding:v1:onboarding-story")).toBe("done");
    await userEvent.click(screen.getByRole("button", { name: "初期ツアーを開く" }));
    await expect(await screen.findByRole("heading", { name: "Zooへようこそ" })).toBeVisible();
  }),
};
export const Skip: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "スキップ" }));
    await expect(localStorage.getItem("zoo:onboarding:v1:onboarding-story")).toBe("done");
  }),
};
export const Mobile: Story = { globals: { viewport: { value: "mobile" } } };

export const SlackSetup: Story = {
  play: playForImplementation(async ({ args }) => {
    await screen.findByRole("dialog");
    for (let i = 0; i < 4; i += 1)
      await userEvent.click(screen.getByRole("button", { name: "次へ" }));
    await expect(screen.getByRole("heading", { name: "Slackと連携する" })).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Slack連携を設定" }));
    await expect(args.onOpenSlack).toHaveBeenCalledOnce();
    await expect(localStorage.getItem("zoo:onboarding:v1:onboarding-story")).toBe("done");
  }),
};
