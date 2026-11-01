import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test";
import { SlackLinkPanel } from "./SlackLinkPanel";

const notLinked = http.get("*/api/slack/link", () =>
  HttpResponse.json({ configured: true, link: null }),
);
const linked = http.get("*/api/slack/link", () =>
  HttpResponse.json({
    configured: true,
    link: {
      team_id: "T1",
      team_name: "Zli",
      slack_user_id: "U1",
      slack_user_name: "alice.with.a.very.long.slack.display.name",
      linked_at: "2026-11-01T09:00:00Z",
    },
  }),
);
const admin = http.get("*/api/users/me", () =>
  HttpResponse.json({
    id: "user-alice",
    user_name: "alice",
    display_name: "Alice",
    is_admin: true,
  }),
);
const manifest = http.get("*/api/admin/slack/manifest", () =>
  HttpResponse.json({
    configured: true,
    manifest: {
      display_information: { name: "Zoo" },
      oauth_config: {
        redirect_urls: ["https://zoo.example/api/slack/oauth/callback"],
        scopes: { user: ["channels:history", "users:read"] },
      },
    },
  }),
);

const meta = {
  title: "Features/Workspace/SlackLink",
  component: SlackLinkPanel,
  args: { navigateTo: fn() },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/settings?tab=slack"] },
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SlackLinkPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

export const NotLinked: Story = {
  parameters: {
    msw: {
      handlers: [
        notLinked,
        http.post("*/api/slack/oauth/start", () =>
          HttpResponse.json({ authorize_url: "https://slack.com/oauth/v2/authorize?state=x" }),
        ),
      ],
    },
  },
  play: async ({ args }) => {
    await userEvent.click(await screen.findByRole("button", { name: "Slack と連携" }));
    await waitFor(() =>
      expect(args.navigateTo).toHaveBeenCalledWith("https://slack.com/oauth/v2/authorize?state=x"),
    );
  },
};

export const Linked: Story = {
  parameters: { msw: { handlers: [linked] } },
  play: async () => {
    await userEvent.click(await screen.findByRole("button", { name: "連携を解除" }));
    await expect(await screen.findByRole("alertdialog")).toBeVisible();
  },
};

export const NotConfigured: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/slack/link", () => HttpResponse.json({ configured: false, link: null })),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByText(/設定されていません/)).toBeVisible();
    await expect(screen.queryByRole("button", { name: "Slack と連携" })).not.toBeInTheDocument();
  },
};

export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [http.get("*/api/slack/link", () => new HttpResponse(null, { status: 500 }))],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  },
};

export const AdminManifest: Story = {
  parameters: { msw: { handlers: [admin, notLinked, manifest] } },
  play: async () => {
    const manifest = await screen.findByRole<HTMLTextAreaElement>("textbox", {
      name: "Slack アプリの manifest",
    });
    await waitFor(() => expect(manifest.value).toContain("/api/slack/oauth/callback"));
  },
};

export const ReturnedWithError: Story = {
  parameters: {
    router: { initialEntries: ["/settings?tab=slack&slack=error&reason=already_linked"] },
    msw: { handlers: [notLinked] },
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "別の Zoo アカウントと連携済み",
    );
  },
};

export const ReturnedFromSlack: Story = {
  parameters: {
    router: { initialEntries: ["/settings?tab=slack&code=abc&state=signed"] },
    msw: {
      handlers: [
        notLinked,
        http.post("*/api/slack/oauth/complete", () =>
          HttpResponse.json({
            configured: true,
            link: {
              team_id: "T1",
              team_name: "Zli",
              slack_user_id: "U1",
              slack_user_name: "alice",
              linked_at: "2026-11-01T09:00:00Z",
            },
          }),
        ),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByText("Zli の alice として連携しています。")).toBeVisible();
  },
};

export const UnlinkError: Story = {
  parameters: {
    msw: {
      handlers: [
        linked,
        http.delete("*/api/slack/link", () =>
          HttpResponse.json({ error: "Slack token revocation failed" }, { status: 502 }),
        ),
      ],
    },
  },
  play: async () => {
    await userEvent.click(await screen.findByRole("button", { name: "連携を解除" }));
    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() => expect(dialog).toBeVisible());
    await userEvent.click(within(dialog).getByRole("button", { name: "連携を解除" }));
    await expect(await screen.findByText(/連携は残っています/)).toBeVisible();
  },
};

export const Mobile: Story = {
  parameters: Linked.parameters,
  globals: { viewport: { value: "mobile" } },
  play: async () => {
    await expect(await screen.findByRole("button", { name: "連携を解除" })).toBeVisible();
  },
};
