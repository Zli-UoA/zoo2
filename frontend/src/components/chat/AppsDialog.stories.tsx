import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent } from "storybook/test";
import { appHandlers } from "../../../.storybook/app-fixtures";
import { StoryShell } from "../../../.storybook/shell";
import { AppsPanel } from "./AppsDialog";
const meta = {
  title: "Features/Apps/Catalog",
  component: AppsPanel,
  parameters: { a11y: { test: "error" }, msw: { handlers: appHandlers } },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="mx-auto h-screen max-w-5xl p-4">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
  args: { onClose: fn(), channelId: "channel-general" },
} satisfies Meta<typeof AppsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Empty: Story = {
  parameters: { msw: { handlers: [http.get("*/api/apps", () => HttpResponse.json([]))] } },
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/apps", async () => {
          await delay("infinite");
          return HttpResponse.json([]);
        }),
      ],
    },
  },
};
export const LoadError: Story = {
  parameters: {
    msw: { handlers: [http.get("*/api/apps", () => new HttpResponse(null, { status: 500 }))] },
  },
};
export const SearchEmpty: Story = {
  play: playForImplementation(async () => {
    await screen.findByRole("button", { name: /Echo Bot/ });
    await userEvent.type(screen.getByRole("textbox", { name: "Appを検索" }), "nonexistent-app");
    await expect(screen.queryByRole("button", { name: /Echo Bot/ })).not.toBeInTheDocument();
  }),
};
export const Details: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: /Echo Bot/ }));
    await expect(screen.getByRole("tab", { name: "管理・開発" })).toBeVisible();
  }),
};
export const OwnerManagement: Story = {
  play: playForImplementation(async (context) => {
    await Details.play?.(context);
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    await expect(screen.getByRole("button", { name: "トークンを発行" })).toBeVisible();
  }),
};
export const OtherOwner: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: /Meeting Notes/ }));
    await expect(screen.queryByRole("tab", { name: "管理・開発" })).not.toBeInTheDocument();
  }),
};
export const Mobile: Story = { ...Details, globals: { viewport: { value: "mobile" } } };
