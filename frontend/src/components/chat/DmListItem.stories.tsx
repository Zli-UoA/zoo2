import { http, HttpResponse } from "msw";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { avatarUsers, userAvatarHandler } from "../../../.storybook/avatar-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn, expect, waitFor } from "storybook/test";
import { storyChannels, storyCollections } from "../../../.storybook/fixtures";
import { DmListItem } from "./DmListItem";
const bobAvatarUrl = avatarUsers[1].avatar_url;
const meta = {
  title: "Features/DMs/ListItem",
  component: DmListItem,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="w-72 bg-sidebar p-2">
        <Story />
      </div>
    ),
  ],
  args: {
    channel: { ...storyChannels[0], id: "dm-story", channel_name: "dm:alice:bob", is_dm: true },
    label: "Bob",
    active: false,
    onOpen: fn(),
  },
} satisfies Meta<typeof DmListItem>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Active: Story = { args: { active: true } };
export const Unread: Story = { args: { unread: true } };
export const LongName: Story = {
  args: { label: "とても長い名前のメンバーとのダイレクトメッセージ" },
};

export const UserAvatar: Story = {
  args: { avatarUrl: bobAvatarUrl },
  parameters: {
    collections: { ...storyCollections, users: avatarUsers },
    msw: { handlers: [userAvatarHandler] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(async () => {
      const image = canvasElement.querySelector<HTMLImageElement>(`img[src="${bobAvatarUrl}"]`);
      await expect(image?.naturalWidth).toBeGreaterThan(0);
    });
  }),
};

export const UserAvatarFallback: Story = {
  args: { avatarUrl: `${bobAvatarUrl}?missing` },
  parameters: {
    msw: { handlers: [http.get(bobAvatarUrl, () => new HttpResponse(null, { status: 404 }))] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(() =>
      expect(canvasElement.querySelector('[data-slot="avatar-fallback"]')).toHaveTextContent("B"),
    );
    await expect(canvasElement.querySelector(`img[src="${bobAvatarUrl}?missing"]`)).toBeNull();
  }),
};
