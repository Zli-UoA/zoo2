import type { Meta, StoryObj } from "@storybook/react-vite";
import { ImagePreview } from "./image-preview";

const meta = {
  title: "Components/Media/ImagePreview",
  component: ImagePreview,
  parameters: { a11y: { test: "error" } },
  args: {
    alt: "Landscape",
    width: 640,
    height: 480,
    blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
    className: "w-64 max-w-full",
    style: { aspectRatio: 4 / 3 },
  },
} satisfies Meta<typeof ImagePreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Loading: Story = {};
export const WithoutMetadata: Story = { args: { blurhash: null } };
export const InvalidHash: Story = { args: { blurhash: "invalid" } };
export const Loaded: Story = {
  args: {
    src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
  },
};
