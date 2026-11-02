import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen } from "storybook/test";
import { AppShell } from "../layout/AppShell";
import { AppConsent } from "./AppConsent";
const app = {
  name: "Release Tracker",
  description: "リリース情報を共有します。",
  icon_url: null,
  owner_id: "user-alice",
  manifest: {
    scopes: ["messages:read", "messages:write"],
    redirect_uris: ["https://example.com/callback"],
  },
};
const meta = {
  title: "Pages/AppConsent",
  component: AppConsent,
  decorators: [
    (Story) => (
      <AppShell>
        <Story />
      </AppShell>
    ),
  ],
  parameters: {
    router: { initialEntries: ["/apps-authorize"] },
    a11y: { test: "error" },
    msw: { handlers: [http.get("*/api/apps/consent-story", () => HttpResponse.json(app))] },
  },
  beforeEach: () => {
    const original = window.location.href;
    const url = new URL(original);
    const params = {
      app_id: "consent-story",
      scope: "messages:read messages:write",
      redirect_uri: "https://example.com/callback",
      code_challenge_method: "S256",
      state: "storybook-state",
      code_challenge: "storybook-challenge",
    };
    for (const [key, value] of Object.entries(params)) url.searchParams.set(key, value);
    window.history.replaceState(null, "", url);
    return () => {
      window.history.replaceState(null, "", original);
    };
  },
} satisfies Meta<typeof AppConsent>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("heading", { name: "Release Tracker" })).toBeVisible();
    await expect(screen.getAllByRole("checkbox")).toHaveLength(2);
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).not.toBeVisible();
  }),
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/apps/consent-story", async () => {
          await delay("infinite");
          return HttpResponse.json(app);
        }),
      ],
    },
  },
};
export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/apps/consent-story", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  }),
};
export const InvalidRequest: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/apps/consent-story", () =>
          HttpResponse.json({ ...app, manifest: { scopes: [], redirect_uris: [] } }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  }),
};
export const Mobile: Story = { ...Default, globals: { viewport: { value: "mobile" } } };
