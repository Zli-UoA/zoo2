import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, waitFor } from "storybook/test";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { ReactionEmoji } from "./ReactionEmoji";
const catalog: KitchenCatalog = {
  emojis: [
    ["1f600", "😀", "grinning face"],
    ["1f431", "🐱", "cat face"],
  ],
  dates: ["20260101"],
  pairs: { "0_1": 0 },
};
const meta = {
  title: "Features/Chat/ReactionEmoji",
  component: ReactionEmoji,
  parameters: { layout: "centered", a11y: { test: "error" } },
  args: { emoji: "🎉", catalog: null },
} satisfies Meta<typeof ReactionEmoji>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Unicode: Story = {};
export const Kitchen: Story = { args: { emoji: "kitchen:1f600_1f431", catalog } };
export const KitchenLoading: Story = { args: { emoji: "kitchen:1f600_1f431" } };
export const CustomMissing: Story = {
  args: { emoji: ":missing:" },
  parameters: {
    msw: {
      handlers: [http.get("*/api/emojis/missing", () => new HttpResponse(null, { status: 404 }))],
    },
  },
};
export const ConsistentSize: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <ReactionEmoji emoji="🎉" catalog={null} />
      <ReactionEmoji emoji="kitchen:1f600_1f431" catalog={catalog} />
      <ReactionEmoji emoji="kitchen:1f600_1f431" catalog={null} />
    </div>
  ),
  play: playForImplementation(async ({ canvasElement }) => {
    const image = canvasElement.querySelector("img")!;
    await waitFor(() => expect(image.naturalWidth).toBeGreaterThan(0));
    for (const element of canvasElement.firstElementChild!.children) {
      await expect(element.getBoundingClientRect().width).toBe(20);
      await expect(element.getBoundingClientRect().height).toBe(20);
    }
    await expect(image).toHaveAttribute("width", "20");
    await expect(image).toHaveAttribute("height", "20");
  }),
};
