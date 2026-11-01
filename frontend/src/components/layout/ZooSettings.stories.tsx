import {
  channelRuleInitialPattern,
  activityPubProfileInitial,
  activityPubProfileSaved,
  activityPubProfileRejectedName,
  channelRuleSavedPattern,
} from "../../../.storybook/preference-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import { http, HttpResponse } from "msw";
import { appHandlers } from "../../../.storybook/app-fixtures";
import { AppShell } from "./AppShell";
import { ZooSettings } from "./ZooSettings";

const readOnlyChannelSettings = http.get("*/api/home/settings", () =>
  HttpResponse.json({ can_manage: false, ltl_channel_pattern: "" }),
);

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
    await userEvent.click(await screen.findByRole("tab", { name: "App・連携" }));
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
    await userEvent.click(await screen.findByRole("tab", { name: "メンバー" }));
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: "管理者にする" }).length).toBeGreaterThan(0),
    );
  }),
};

export const GeneralMobile: Story = { ...General, globals: { viewport: { value: "mobile" } } };

let savedPattern = channelRuleInitialPattern;
export const ChannelRule: Story = {
  beforeEach: () => {
    savedPattern = channelRuleInitialPattern;
  },
  parameters: {
    msw: {
      handlers: [
        ...appHandlers,
        http.get("*/api/activitypub/profile", () =>
          HttpResponse.json({
            display_name: activityPubProfileInitial.display_name,
            default_display_name: "Alice",
            summary: activityPubProfileInitial.summary,
          }),
        ),
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
  play: playForImplementation(
    async () => {
      await userEvent.click(await screen.findByRole("tab", { name: "ActivityPub" }));
      const field = await screen.findByRole("textbox", { name: "LTLに流すチャンネル名のルール" });
      await userEvent.clear(field);
      await userEvent.type(field, channelRuleSavedPattern);
      await userEvent.click(
        within(
          screen.getByRole("textbox", { name: "LTLに流すチャンネル名のルール" }).closest("form")!,
        ).getByRole("button", { name: "保存" }),
      );
      await waitFor(() => expect(savedPattern).toBe(channelRuleSavedPattern));
      await expect(field).toHaveValue(channelRuleSavedPattern);
    },
    async () => {
      await waitFor(() => expect(savedPattern).toBe(channelRuleSavedPattern));
    },
  ),
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
        http.get("*/api/activitypub/profile", () =>
          HttpResponse.json({
            display_name: activityPubProfileInitial.display_name,
            default_display_name: "Alice",
            summary: activityPubProfileInitial.summary,
          }),
        ),
        http.get("*/api/home/settings", () =>
          ++ruleLoads === 1
            ? new HttpResponse(null, { status: 500 })
            : HttpResponse.json({
                can_manage: true,
                ltl_channel_pattern: channelRuleInitialPattern,
              }),
        ),
        http.put("*/api/home/settings", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: playForImplementation(
    async () => {
      await userEvent.click(await screen.findByRole("tab", { name: "ActivityPub" }));
      await expect(await screen.findByRole("alert")).toBeVisible();
      await userEvent.click(screen.getByRole("button", { name: "更新" }));
      await expect(
        await screen.findByRole("textbox", { name: "LTLに流すチャンネル名のルール" }),
      ).toBeVisible();
      await userEvent.click(
        within(
          screen.getByRole("textbox", { name: "LTLに流すチャンネル名のルール" }).closest("form")!,
        ).getByRole("button", { name: "保存" }),
      );
      await expect(await screen.findByRole("alert")).toBeVisible();
    },
    async () => {
      await expect(ruleLoads).toBe(2);
    },
  ),
};

let savedProfile = {
  display_name: activityPubProfileInitial.display_name,
  summary: activityPubProfileInitial.summary,
  default_display_name: "Alice",
};
const profileHandlers = [
  ...appHandlers,
  readOnlyChannelSettings,
  http.get("*/api/activitypub/profile", () => HttpResponse.json(savedProfile)),
  http.put("*/api/activitypub/profile", async ({ request }) => {
    const body = (await request.json()) as { display_name: string; summary: string };
    savedProfile = { ...savedProfile, ...body, display_name: body.display_name.trim() };
    return HttpResponse.json(savedProfile);
  }),
];
export const ActivityPub: Story = {
  beforeEach: () => {
    savedProfile = {
      display_name: activityPubProfileInitial.display_name,
      summary: activityPubProfileInitial.summary,
      default_display_name: "Alice",
    };
  },
  parameters: { msw: { handlers: profileHandlers } },
  play: playForImplementation(
    async () => {
      await userEvent.click(await screen.findByRole("tab", { name: "ActivityPub" }));
      const field = await screen.findByRole("textbox", { name: "APub表示名" });
      await expect(field).toHaveValue(activityPubProfileInitial.display_name);
      await userEvent.clear(field);
      await userEvent.type(field, activityPubProfileSaved.display_name);
      const bio = screen.getByRole("textbox", { name: "自己紹介" });
      await userEvent.clear(bio);
      await userEvent.type(bio, activityPubProfileSaved.summary);
      await userEvent.click(within(field.closest("form")!).getByRole("button", { name: "保存" }));
      await expect(await screen.findByRole("status")).toHaveTextContent("保存しました");
      await expect(savedProfile.display_name).toBe(activityPubProfileSaved.display_name);
      await expect(savedProfile.summary).toBe(activityPubProfileSaved.summary);
      await userEvent.clear(field);
      await userEvent.click(within(field.closest("form")!).getByRole("button", { name: "保存" }));
      await waitFor(() => expect(savedProfile.display_name).toBe(""));
      await expect(field).toHaveAttribute("placeholder", "Alice");
    },
    async () => {
      await waitFor(() => expect(savedProfile.display_name).toBe(""));
      await expect(savedProfile.summary).toBe(activityPubProfileSaved.summary);
    },
  ),
};
export const ActivityPubMobile: Story = {
  ...ActivityPub,
  globals: { viewport: { value: "mobile" } },
};
export const ActivityPubLoading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/activitypub/profile", () => new Promise(() => {})),
        readOnlyChannelSettings,
        ...appHandlers,
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("tab", { name: "ActivityPub" }));
    await expect(await screen.findByRole("status")).toHaveTextContent("読み込み中");
  }),
};
export const ActivityPubError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/activitypub/profile", () => new HttpResponse(null, { status: 500 })),
        readOnlyChannelSettings,
        ...appHandlers,
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("tab", { name: "ActivityPub" }));
    await expect(await screen.findByRole("alert")).toBeVisible();
  }),
};
export const ActivityPubSaveError: Story = {
  beforeEach: ActivityPub.beforeEach,
  parameters: {
    msw: {
      handlers: [
        http.put("*/api/activitypub/profile", () => new HttpResponse(null, { status: 500 })),
        ...profileHandlers,
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("tab", { name: "ActivityPub" }));
    const field = await screen.findByRole("textbox", { name: "APub表示名" });
    await userEvent.clear(field);
    await userEvent.type(field, activityPubProfileRejectedName);
    await userEvent.click(within(field.closest("form")!).getByRole("button", { name: "保存" }));
    await expect(await screen.findByRole("alert")).toBeVisible();
    await expect(field).toHaveValue(activityPubProfileRejectedName);
  }),
};
