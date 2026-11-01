import { playForImplementation } from "../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, waitFor } from "storybook/test";
import { AppBlocks } from "../components/chat/AppBlocks";
let payloads: unknown[] = [];
const meta = {
  title: "Flows/Apps/SubmitAndRetry",
  component: AppBlocks,
  beforeEach: () => {
    payloads = [];
  },
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.post("*/api/apps/:id/invoke", async ({ request }) => {
          payloads.push(await request.json());
          return payloads.length === 1
            ? new HttpResponse(null, { status: 500 })
            : HttpResponse.json({ event_id: "event-1" });
        }),
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
    submit: true,
    onDone: fn(),
    blocks: [
      { type: "input", action_id: "title", label: "通知のタイトル", required: true },
      { type: "select", action_id: "priority", label: "優先度", options: ["通常", "高"] },
    ],
  },
} satisfies Meta<typeof AppBlocks>;
export default meta;
type Story = StoryObj<typeof meta>;
export const RetryPreservesValues: Story = {
  play: playForImplementation(
    async () => {
      await userEvent.type(await screen.findByRole("textbox"), "リリース完了");
      await userEvent.selectOptions(screen.getByRole("combobox"), "高");
      await userEvent.click(screen.getByRole("button"));
      await expect(await screen.findByRole("alert")).toBeVisible();
      await expect(screen.getByRole("textbox")).toHaveValue("リリース完了");
      await expect(screen.getByRole("combobox")).toHaveValue("高");
      await userEvent.click(screen.getByRole("button"));
    },
    async (context) => {
      await waitFor(() => expect(context.args.onDone).toHaveBeenCalledOnce());
      if (context.globals.implementation !== "kotlin")
        await expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      await expect(payloads).toHaveLength(2);
      await expect(payloads[0]).toMatchObject({
        kind: "submit",
        values: { title: "リリース完了", priority: "高" },
      });
      await expect(payloads[1]).toEqual(payloads[0]);
    },
  ),
};
