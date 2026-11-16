import { slackNotLinkedHandler } from "../../../.storybook/slack-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import { useRouterState } from "@tanstack/react-router";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { AppShell } from "./AppShell";
import { ZooSettings } from "./ZooSettings";
import { ChannelScreen } from "../chat/ChannelScreen";
import { NotificationsScreen } from "../chat/NotificationsScreen";
import { WelcomePanel } from "../chat/WelcomePanel";

/** The real route screens change with the same router as the running app. */
function TourScreen() {
  const path = useRouterState({ select: (state) => state.location.pathname });
  if (path === "/settings") return <ZooSettings />;
  if (path === "/notifications") return <NotificationsScreen />;
  if (path.startsWith("/channels/")) return <ChannelScreen channelId={path.split("/")[2]} />;
  return <WelcomePanel />;
}
const meta = {
  title: "Flows/Onboarding/Tour",
  component: AppShell,
  parameters: { a11y: { test: "error" }, msw: { handlers: [slackNotLinkedHandler] } },
  args: { children: <TourScreen /> },
  beforeEach: () => {
    localStorage.removeItem("zoo:onboarding:v1:user-alice");
  },
} satisfies Meta<typeof AppShell>;
export default meta;
type Story = StoryObj<typeof meta>;
const panel = () => within(screen.getByRole("complementary", { name: "使い方ツアー" }));
async function nextTo(title: string) {
  await userEvent.click(panel().getByRole("button", { name: "次へ" }));
  await waitFor(() => expect(panel().getByRole("heading", { name: title })).toBeVisible());
}
async function chooseChannel() {
  await nextTo("チャンネルを探す");
  await expect(panel().getByRole("button", { name: "次へ" })).toBeDisabled();
  await userEvent.click(await screen.findByRole("link", { name: "general" }));
  await waitFor(() => expect(panel().getByRole("button", { name: "次へ" })).toBeEnabled());
}
export const FirstVisit: Story = {};
export const CompleteAndReplay: Story = {
  play: playForImplementation(async () => {
    await screen.findByRole("complementary", { name: "使い方ツアー" });
    await chooseChannel();
    await nextTo("会話に参加する");
    await expect(await screen.findByRole("textbox", { name: "メッセージを送信" })).toBeVisible();
    await nextTo("通知を確認する");
    await nextTo("Slackと連携する");
    await nextTo("自分に合う設定にする");
    await expect(screen.getByRole("tab", { name: "一般" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await userEvent.click(panel().getByRole("button", { name: "Zooを使い始める" }));
    await expect(localStorage.getItem("zoo:onboarding:v1:user-alice")).toBe("done");
    // Settings has a back button; the rail and its replay control belong to the channel screen.
    // Follow the real history from General through Slack back to Notifications.
    await userEvent.click(screen.getByRole("button", { name: "チャンネル一覧へ戻る" }));
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Slack 連携" })).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );
    await userEvent.click(screen.getByRole("button", { name: "チャンネル一覧へ戻る" }));
    await userEvent.click(await screen.findByRole("button", { name: "初期ツアーを開く" }));
    await expect(panel().getByRole("heading", { name: "Zooへようこそ" })).toBeVisible();
  }),
};
export const Skip: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "スキップ" }));
    await expect(localStorage.getItem("zoo:onboarding:v1:user-alice")).toBe("done");
  }),
};
export const Mobile: Story = { globals: { viewport: { value: "mobile" } } };
export const SlackSetup: Story = {
  play: playForImplementation(async () => {
    await screen.findByRole("complementary", { name: "使い方ツアー" });
    await chooseChannel();
    await nextTo("会話に参加する");
    await nextTo("通知を確認する");
    await nextTo("Slackと連携する");
    await expect(screen.getByRole("tab", { name: "Slack 連携" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(await screen.findByRole("button", { name: "Slack と連携" })).toBeVisible();
    await expect(panel().getByRole("heading", { name: "Slackと連携する" })).toBeVisible();
    await userEvent.click(panel().getByRole("button", { name: "戻る" }));
    await expect(await screen.findByRole("region", { name: "通知" })).toBeVisible();
  }),
};
