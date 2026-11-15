import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent } from "storybook/test";
import { DefaultChannelsSettings } from "./DefaultChannelsSettings";

const initial = {
  channel_ids: ["general"],
  channels: [
    { id: "general", channel_name: "general" },
    { id: "help", channel_name: "help" },
  ],
};
const load = http.get("*/api/workspace/default-channels", () => HttpResponse.json(initial));
const meta = {
  title: "Features/Workspace/DefaultChannels",
  component: DefaultChannelsSettings,
  parameters: { a11y: { test: "error" }, msw: { handlers: [load] } },
} satisfies Meta<typeof DefaultChannelsSettings>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {
  parameters: {
    msw: {
      handlers: [
        load,
        http.patch("*/api/workspace/default-channels", async ({ request }) => {
          const body = await request.json();
          await expect(body).toEqual({ channel_ids: ["general", "help"] });
          return HttpResponse.json({ ...initial, channel_ids: ["general", "help"] });
        }),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("checkbox", { name: "#general" })).toBeChecked();
    await userEvent.click(screen.getByRole("checkbox", { name: "#help" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await expect(await screen.findByRole("status")).toHaveTextContent("保存しました。");
  },
};
export const Empty: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/workspace/default-channels", () =>
          HttpResponse.json({ channel_ids: [], channels: [] }),
        ),
      ],
    },
  },
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/workspace/default-channels", async () => {
          await delay("infinite");
        }),
      ],
    },
  },
};
export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/workspace/default-channels", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
};
export const SaveError: Story = {
  parameters: {
    msw: {
      handlers: [
        load,
        http.patch(
          "*/api/workspace/default-channels",
          () => new HttpResponse(null, { status: 500 }),
        ),
      ],
    },
  },
  play: async () => {
    await screen.findByRole("checkbox", { name: "#general" });
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await expect(await screen.findByRole("alert")).toBeVisible();
  },
};
