import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { HomePostContent } from "../home/HomePostContent";
import { SensitiveMedia } from "./sensitive-media";

const meta = {
  title: "Components/Media/SensitiveMedia",
  component: SensitiveMedia,
  parameters: { a11y: { test: "error" } },
  args: { sensitive: true, children: <p>Image content</p> },
} satisfies Meta<typeof SensitiveMedia>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Hidden: Story = {};
export const RevealAndHide: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.queryByText("Image content")).not.toBeInTheDocument();
    await userEvent.click(canvas.getByRole("button", { name: "画像を表示" }));
    await expect(canvas.getByText("Image content")).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: "画像を隠す" }));
    await expect(canvas.queryByText("Image content")).not.toBeInTheDocument();
  },
};
export const ActivityPub: Story = {
  render: () => (
    <div className="w-64 max-w-full">
      <HomePostContent
        content="Remote sensitive photo"
        media={[
          {
            url: "https://peer.example/image.png",
            image_id: "a".repeat(64),
            media_type: "image/png",
            name: "Remote photo",
            is_sensitive: true,
            blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
            width: 640,
            height: 480,
          },
        ]}
      />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.queryByRole("img")).not.toBeInTheDocument();
    await expect(canvas.queryByRole("link")).not.toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "画像を表示" })).toBeVisible();
  },
};
