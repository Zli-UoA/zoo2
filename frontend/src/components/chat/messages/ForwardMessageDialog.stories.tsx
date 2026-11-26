import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import { storySurfaces } from "../../../../.storybook/story-surfaces";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../../.storybook/shell";
import type { ResolvedForwardMessage } from "../../../lib/api/types";
import { ForwardMessageDialog, type ForwardDestination } from "./ForwardMessageDialog";

const destinations: ForwardDestination[] = [
  { id: "ch-general", label: "general", isDm: false, isPrivate: false },
  { id: "ch-dev", label: "dev-ops", isDm: false, isPrivate: true },
  { id: "ch-dm", label: "Alice", isDm: true, isPrivate: true },
];

const source: ResolvedForwardMessage = {
  id: "msg-src-1",
  channel_id: "ch-general",
  channel_name: "general",
  is_private: false,
  is_dm: false,
  dm_participants: [],
  parent_id: null,
  user_id: "user-1",
  created_at: "2026-01-02 20:20:00.000",
  updated_at: null,
  content: "明日の定例は 10:00 から開始します。資料の確認をお願いします。",
  card: null,
  components: null,
  message_kind: "message",
  author: {
    id: "user-1",
    user_name: "alice",
    display_name: "Alice",
    avatar_url: null,
    is_bot: false,
    app_id: null,
  },
  emojis: {},
  attachments: [],
  has_forwards: false,
};

const meta = {
  title: "Features/Chat/ForwardMessageDialog",
  component: ForwardMessageDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    open: { control: "boolean" },
    isSubmitting: { control: "boolean" },
  },
  args: {
    open: true,
    onOpenChange: fn(),
    source,
    destinations,
    isSubmitting: false,
    error: null,
    onSubmit: fn(),
  },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className={storySurfaces["Components/Dialog"]}>
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ForwardMessageDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    await expect(within(dialog).getByRole("button", { name: "転送する" })).toBeDisabled();
    await waitFor(() =>
      expect(within(dialog).getByRole("option", { name: "general" })).toBeVisible(),
    );
  }),
};

export const SelectedDestination: Story = {
  args: {
    defaultSelectedId: "ch-dev",
    defaultComment: "共有します！",
  },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    await waitFor(() => expect(within(dialog).getByText("dev-ops")).toBeVisible());
    await expect(within(dialog).getByRole("button", { name: "転送する" })).toBeEnabled();
  }),
};

export const SearchEmpty: Story = {
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    await userEvent.type(
      within(dialog).getByRole("combobox", { name: "転送先" }),
      "no-such-channel",
    );
    await waitFor(() =>
      expect(within(dialog).getByText("一致するチャンネルがありません。")).toBeVisible(),
    );
  }),
};

export const NoDestinations: Story = {
  args: {
    destinations: [],
  },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    await waitFor(() =>
      expect(within(dialog).getByText("転送できるチャンネルまたは DM がありません")).toBeVisible(),
    );
    await expect(within(dialog).getByRole("button", { name: "転送する" })).toBeDisabled();
  }),
};

export const Submitting: Story = {
  args: {
    defaultSelectedId: "ch-general",
    isSubmitting: true,
  },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    const btn = within(dialog).getByRole("button", { name: "転送中…" });
    await expect(btn).toBeDisabled();
    await expect(btn).toHaveAttribute("aria-busy", "true");
  }),
};

export const Error: Story = {
  args: {
    defaultSelectedId: "ch-general",
    error: "元のメッセージが存在しないか、転送する権限がありません",
  },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    await expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "元のメッセージが存在しないか、転送する権限がありません",
    );
  }),
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    defaultSelectedId: "ch-dm",
  },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "このメッセージを転送する" });
    await expect(within(dialog).getByRole("button", { name: "転送する" })).toBeEnabled();
  }),
};
