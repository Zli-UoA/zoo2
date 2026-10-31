import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";
import { http, HttpResponse } from "msw";
import { appHandlers } from "../../../.storybook/app-fixtures";
import { StoryShell } from "../../../.storybook/shell";
import { ZooSettings } from "./ZooSettings";

const meta = {
  title: "Pages/ZooSettings",
  component: ZooSettings,
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/settings?channel=channel-general"] },
    msw: {
      handlers: appHandlers,
    },
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general">
        <div className="h-screen">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ZooSettings>;
export default meta;
type Story = StoryObj<typeof meta>;

export const General: Story = {
  play: async () => {
    await waitFor(() => expect(screen.getByRole("heading", { name: "Zoo設定" })).toBeVisible());
  },
};
export const Apps: Story = {
  play: async () => {
    await userEvent.click(screen.getByRole("tab", { name: "App・連携" }));
    await userEvent.click(await screen.findByRole("button", { name: /Echo Bot/ }));
    await expect(screen.getByRole("button", { name: "メッセージ" })).toBeVisible();
  },
};
export const AppsMobile: Story = { ...Apps, globals: { viewport: { value: "mobile" } } };
export const Management: Story = {
  play: async (context) => {
    await Apps.play?.(context);
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    await expect(screen.getByRole("button", { name: "トークンを発行" })).toBeVisible();
  },
};

export const Members: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/users/me", () =>
          HttpResponse.json({
            id: "user-alice",
            user_name: "alice",
            display_name: "Alice",
            is_admin: true,
          }),
        ),
        http.get("*/api/users/admins", () =>
          HttpResponse.json({ user_ids: ["user-alice", "user-admin"], protected_user_ids: [] }),
        ),
      ],
    },
  },
  play: async () => {
    await userEvent.click(screen.getByRole("tab", { name: "メンバー" }));
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: "管理者にする" }).length).toBeGreaterThan(0),
    );
  },
};
