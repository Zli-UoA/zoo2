import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { storyDeploymentCard } from "../../../.storybook/app-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent } from "storybook/test";
import { AppBlocks } from "./AppBlocks";
const meta = {
  title: "Features/Apps/Blocks",
  component: AppBlocks,
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.post("*/api/apps/:id/invoke", () => HttpResponse.json({ event_id: "event-1" })),
      ],
    },
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-md p-4">
        <Story />
      </div>
    ),
  ],
  args: {
    appId: "echo",
    surfaceId: "surface-1",
    submit: true,
    blocks: [
      { type: "text", text: "リリース通知を作成します。" },
      { type: "input", action_id: "title", label: "通知のタイトル", required: true },
      { type: "select", action_id: "priority", label: "優先度", options: ["通常", "高"] },
    ],
  },
} satisfies Meta<typeof AppBlocks>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Pending: Story = {
  parameters: {
    msw: {
      handlers: [
        http.post("*/api/apps/:id/invoke", async () => {
          await delay("infinite");
          return HttpResponse.json({});
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.type(await screen.findByRole("textbox"), "リリース完了");
    await userEvent.click(screen.getByRole("button"));
    await expect(screen.getByRole("button")).toBeDisabled();
  }),
};
export const Failed: Story = {
  parameters: {
    msw: {
      handlers: [http.post("*/api/apps/:id/invoke", () => new HttpResponse(null, { status: 500 }))],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.type(await screen.findByRole("textbox"), "リリース完了");
    await userEvent.click(screen.getByRole("button"));
    await expect(await screen.findByRole("alert")).toBeVisible();
    await expect(screen.getByRole("textbox")).toHaveValue("リリース完了");
  }),
};
export const InvalidBlocks: Story = {
  args: {
    submit: false,
    blocks: [
      { type: "unsupported" },
      { type: "text", text: "未対応のブロックがあっても本文を表示します。" },
    ],
  },
};

export const Card: Story = {
  args: {
    submit: false,
    blocks: [{ type: "card", card: storyDeploymentCard }],
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("link", { name: storyDeploymentCard.title }),
    ).toBeVisible();
    await expect(
      screen.getByRole("link", { name: "3968f64 (fix/github-bot-paused-queue)" }),
    ).toHaveClass("text-link");
  }),
};
