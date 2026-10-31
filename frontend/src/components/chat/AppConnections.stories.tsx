import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen } from "storybook/test";
import { StoryShell } from "../../../.storybook/shell";
import { AppConnections } from "./AppConnections";
const connection = (state: string) =>
  http.get("*/api/apps/:id/connection", () => HttpResponse.json({ state }));
const meta = {
  title: "Features/Apps/Connections",
  component: AppConnections,
  parameters: { a11y: { test: "error" }, msw: { handlers: [connection("disconnected")] } },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="mx-auto max-w-xl p-4">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
  args: { appId: "echo", connections: true, assistant: false },
} satisfies Meta<typeof AppConnections>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Disconnected: Story = {};
export const Connected: Story = { parameters: { msw: { handlers: [connection("connected")] } } };
export const Requested: Story = { parameters: { msw: { handlers: [connection("requested")] } } };
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/apps/:id/connection", async () => {
          await delay("infinite");
          return HttpResponse.json({ state: "disconnected" });
        }),
      ],
    },
  },
};
export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/apps/:id/connection", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  }),
};
export const AssistantSession: Story = {
  args: { assistant: true },
  parameters: {
    msw: {
      handlers: [
        connection("connected"),
        http.get("*/api/assistant-sessions", () =>
          HttpResponse.json([
            { id: "session-1", app_id: "echo", title: "リリースの相談", state: "running" },
          ]),
        ),
        http.post("*/api/apps/:id/context", () => HttpResponse.json({})),
      ],
    },
  },
};
