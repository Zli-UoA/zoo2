import { storyCollections } from "../../../.storybook/fixtures";
import { avatarUsers, userAvatarHandler, userAvatarUrl } from "../../../.storybook/avatar-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, within, waitFor } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { LeftRail } from "./LeftRail";

const meta = {
  title: "Features/Navigation/Rail",
  component: LeftRail,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="flex h-[640px] bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof LeftRail>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    const toggle = within(canvasElement).queryByRole("button", {
      name: "Compose版を使う（実験・開発中）",
    });
    await expect(toggle).not.toBeInTheDocument();
  }),
};

export const Mobile: Story = {
  ...Default,
  globals: { viewport: { value: "mobile" } },
};

export const UserAvatar: Story = {
  parameters: {
    collections: { ...storyCollections, users: avatarUsers },
    msw: { handlers: [userAvatarHandler] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(async () => {
      const image = canvasElement.querySelector<HTMLImageElement>(`img[src="${userAvatarUrl}"]`);
      await expect(image?.naturalWidth).toBeGreaterThan(0);
    });
  }),
};

export const AboutZoo: Story = {
  beforeEach: () => {
    document.documentElement.dataset.zooBuildSha = "0123456789abcdef0123456789abcdef01234567";
    return () => {
      delete document.documentElement.dataset.zooBuildSha;
    };
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const trigger = within(canvasElement).getByRole("button", { name: "Zooについて" });
    await userEvent.click(trigger);

    const dialog = await screen.findByRole("dialog", { name: "Zooについて" });
    await waitFor(async () => {
      await expect(
        within(dialog).getByText("バージョン: 0123456789ab", { exact: true }),
      ).toBeVisible();
    });

    await userEvent.keyboard("{Escape}");
    await waitFor(async () => {
      await expect(screen.queryByRole("dialog", { name: "Zooについて" })).not.toBeInTheDocument();
      await expect(trigger).toHaveFocus();
    });

    await userEvent.keyboard("{Enter}");
    await waitFor(async () => {
      await expect(screen.getByRole("dialog", { name: "Zooについて" })).toBeVisible();
    });
  }),
};

export const AboutZooUnknownVersion: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    const trigger = within(canvasElement).getByRole("button", { name: "Zooについて" });
    await userEvent.click(trigger);

    const dialog = await screen.findByRole("dialog", { name: "Zooについて" });
    await waitFor(async () => {
      await expect(within(dialog).getByText("バージョン: 不明", { exact: true })).toBeVisible();
    });
  }),
};
