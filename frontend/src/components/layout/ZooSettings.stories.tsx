import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";
import { http, HttpResponse } from "msw";
import { appHandlers } from "../../../.storybook/app-fixtures";
import { AppShell } from "./AppShell";
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
      <AppShell>
        <Story />
      </AppShell>
    ),
  ],
} satisfies Meta<typeof ZooSettings>;
export default meta;
type Story = StoryObj<typeof meta>;

export const General: Story = {
  play: playForImplementation(async () => {
    await waitFor(() => expect(screen.getByRole("heading", { name: "Zoo設定" })).toBeVisible());
    const bounds = screen.getByRole("region", { name: "Zoo設定" }).getBoundingClientRect();
    await expect(bounds.x).toBe(0);
    await expect(bounds.y).toBe(0);
    await expect(bounds.width).toBe(document.documentElement.clientWidth);
    await expect(bounds.height).toBe(window.innerHeight);
    await expect(screen.getByRole("group", { name: "テーマ" })).toBeVisible();
    await expect(screen.getByRole("combobox", { name: "言語" })).toBeVisible();
    await expect(screen.getByRole("combobox", { name: "プッシュ通知" })).toBeDisabled();
  }),
};
export const Apps: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(screen.getByRole("tab", { name: "App・連携" }));
    await userEvent.click(await screen.findByRole("button", { name: /Echo Bot/ }));
    await expect(screen.getByRole("button", { name: "メッセージ" })).toBeVisible();
  }),
};
export const AppsMobile: Story = { ...Apps, globals: { viewport: { value: "mobile" } } };
export const Management: Story = {
  play: playForImplementation(async (context) => {
    await Apps.play?.(context);
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    await expect(screen.getByRole("button", { name: "トークンを発行" })).toBeVisible();
  }),
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
  play: playForImplementation(async () => {
    await userEvent.click(screen.getByRole("tab", { name: "メンバー" }));
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: "管理者にする" }).length).toBeGreaterThan(0),
    );
  }),
};

export const GeneralMobile: Story = { ...General, globals: { viewport: { value: "mobile" } } };
