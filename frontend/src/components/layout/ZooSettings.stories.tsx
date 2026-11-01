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
  play: async () => {
    await waitFor(() => expect(screen.getByRole("heading", { name: "Zoo設定" })).toBeVisible());
    const bounds = screen.getByRole("region", { name: "Zoo設定" }).getBoundingClientRect();
    await expect(bounds.x).toBe(0);
    await expect(bounds.y).toBe(0);
    await expect(bounds.width).toBe(document.documentElement.clientWidth);
    await expect(bounds.height).toBe(window.innerHeight);
    await expect(screen.getByRole("group", { name: "テーマ" })).toBeVisible();
    await expect(screen.getByRole("combobox", { name: "言語" })).toBeVisible();
    await expect(screen.getByRole("combobox", { name: "プッシュ通知" })).toBeDisabled();
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

export const GeneralMobile: Story = { ...General, globals: { viewport: { value: "mobile" } } };

let savedPattern = "ltl-%";
export const ChannelRule: Story = {
  beforeEach: () => {
    savedPattern = "ltl-%";
  },
  parameters: {
    msw: {
      handlers: [
        ...appHandlers,
        http.get("*/api/home/settings", () =>
          HttpResponse.json({ can_manage: true, ltl_channel_pattern: savedPattern }),
        ),
        http.put("*/api/home/settings", async ({ request }) => {
          const body = (await request.json()) as { ltl_channel_pattern: string };
          savedPattern = body.ltl_channel_pattern;
          return HttpResponse.json({ ltl_channel_pattern: savedPattern });
        }),
      ],
    },
  },
  play: async () => {
    const field = await screen.findByRole("textbox", { name: "LTLに流すチャンネル名のルール" });
    await userEvent.clear(field);
    await userEvent.type(field, "public-%");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(savedPattern).toBe("public-%"));
    await expect(field).toHaveValue("public-%");
  },
};

let ruleLoads = 0;
export const ChannelRuleError: Story = {
  beforeEach: () => {
    ruleLoads = 0;
  },
  parameters: {
    msw: {
      handlers: [
        ...appHandlers,
        http.get("*/api/home/settings", () =>
          ++ruleLoads === 1
            ? new HttpResponse(null, { status: 500 })
            : HttpResponse.json({ can_manage: true, ltl_channel_pattern: "ltl-%" }),
        ),
        http.put("*/api/home/settings", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    await expect(
      await screen.findByRole("textbox", { name: "LTLに流すチャンネル名のルール" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await expect(await screen.findByRole("alert")).toBeVisible();
  },
};

export const SlackImport: Story = {
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
        http.get("*/api/admin/slack-imports", () => HttpResponse.json({ imports: [] })),
        ...appHandlers,
      ],
    },
  },
  play: async () => {
    await userEvent.click(await screen.findByRole("tab", { name: "Slack 取り込み" }));
    await expect(
      await screen.findByRole("button", { name: "ZIP を選んでアップロード" }),
    ).toBeVisible();
  },
};
