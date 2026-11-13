import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { storyChannels } from "../../../.storybook/fixtures";
import { ChannelBrowserDialog } from "./ChannelBrowserDialog";

const joined = new Set(["channel-general", "channel-design", "channel-secret"]);

const meta = {
  title: "Features/Channels/Discovery",
  component: ChannelBrowserDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    open: { control: "boolean" },
  },
  args: {
    open: true,
    onOpenChange: fn(),
    channels: storyChannels,
    joinedChannelIds: joined,
    onJoinChannel: fn(async () => {}),
  },
  decorators: [
    (Story) => (
      <StoryShell>
        <Story />
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelBrowserDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async () => {
    await expect(await screen.findByText("random")).toBeInTheDocument();
    await expect(screen.getByRole("link", { name: /random/ })).toBeInTheDocument();
    await expect(screen.getAllByRole("button", { name: "参加する" }).length).toBeGreaterThan(0);
  }),
};

export const JoinStaysInDialog: Story = {
  play: playForImplementation(async ({ args }) => {
    await userEvent.click((await screen.findAllByRole("button", { name: "参加する" }))[0]);
    await expect(args.onJoinChannel).toHaveBeenCalledWith("channel-random");
    await expect(args.onOpenChange).not.toHaveBeenCalled();
  }),
};

export const JoinPending: Story = {
  args: { onJoinChannel: fn(() => new Promise<void>(() => {})) },
  play: playForImplementation(async () => {
    await userEvent.click((await screen.findAllByRole("button", { name: "参加する" }))[0]);
    const pending = await screen.findByRole("button", { name: "参加中…" });
    await expect(pending).toBeDisabled();
    await expect(pending).toHaveAttribute("aria-busy", "true");
  }),
};

export const JoinFailed: Story = {
  args: { onJoinChannel: fn(() => Promise.reject(new Error("network error"))) },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click((await screen.findAllByRole("button", { name: "参加する" }))[0]);
    await expect(await screen.findByRole("alert")).toBeInTheDocument();
    await expect(args.onOpenChange).not.toHaveBeenCalled();
    await expect(screen.getAllByRole("button", { name: "参加する" })[0]).toBeEnabled();
  }),
};

export const SortByName: Story = {
  play: playForImplementation(async () => {
    const rows = async () =>
      (await screen.findAllByRole("link")).map((link) => link.textContent ?? "");
    await userEvent.click(
      await screen.findByRole("button", { name: "アーカイブ済みのチャンネルを含める" }),
    );
    await userEvent.click(screen.getByRole("button", { name: /並び替え/ }));
    await userEvent.click(await screen.findByRole("menuitemradio", { name: /A → Z/ }));
    await expect((await rows())[0]).toContain("old-announcements");
    await userEvent.click(screen.getByRole("button", { name: /並び替え/ }));
    await userEvent.click(await screen.findByRole("menuitemradio", { name: /Z → A/ }));
    await expect((await rows())[0]).toContain("random");
  }),
};

export const IncludeArchived: Story = {
  play: playForImplementation(async () => {
    await expect(await screen.findByText("random")).toBeInTheDocument();
    await expect(screen.queryByText("old-announcements")).not.toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "アーカイブ済みのチャンネルを含める" });
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(toggle);
    await expect(await screen.findByText("old-announcements")).toBeInTheDocument();
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
  }),
};

export const SearchEmpty: Story = {
  play: playForImplementation(async () => {
    await userEvent.type(await screen.findByRole("searchbox"), "zzzz");
    await expect(screen.getByText("一致するチャンネルがありません。")).toBeInTheDocument();
  }),
};

export const Closed: Story = {
  args: {
    open: false,
  },
};
