import { avatarStoryInitials } from "../../../.storybook/basic-fixtures";
import { storySurfaces } from "../../../.storybook/story-surfaces";
import type { Meta, StoryObj } from "@storybook/react-vite";

import { Avatar, AvatarBadge, AvatarFallback, AvatarGroup, AvatarGroupCount } from "./avatar";

const meta = {
  title: "Components/Avatar",
  component: Avatar,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    size: { control: "select", options: ["sm", "default", "lg"] },
  },
  args: {
    size: "default",
  },
  decorators: [
    (Story) => (
      <div className={storySurfaces["Components/Avatar"]}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Avatar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <Avatar {...args}>
      <AvatarFallback>{avatarStoryInitials.default}</AvatarFallback>
    </Avatar>
  ),
};

export const Sizes: Story = {
  render: () => (
    <div className="flex items-end gap-3">
      {(["sm", "default", "lg"] as const).map((size) => (
        <Avatar key={size} size={size}>
          <AvatarFallback>
            {avatarStoryInitials.sizes[size === "sm" ? 0 : size === "lg" ? 2 : 1]}
          </AvatarFallback>
        </Avatar>
      ))}
    </div>
  ),
};

export const Badge: Story = {
  render: () => (
    <Avatar>
      <AvatarFallback>{avatarStoryInitials.badge}</AvatarFallback>
      <AvatarBadge />
    </Avatar>
  ),
};

export const Group: Story = {
  render: () => (
    <AvatarGroup>
      <Avatar size="sm">
        <AvatarFallback>{avatarStoryInitials.group[0]}</AvatarFallback>
      </Avatar>
      <Avatar size="sm">
        <AvatarFallback>{avatarStoryInitials.group[1]}</AvatarFallback>
      </Avatar>
      <Avatar size="sm">
        <AvatarFallback>{avatarStoryInitials.group[2]}</AvatarFallback>
      </Avatar>
      <AvatarGroupCount>{avatarStoryInitials.count}</AvatarGroupCount>
    </AvatarGroup>
  ),
};
