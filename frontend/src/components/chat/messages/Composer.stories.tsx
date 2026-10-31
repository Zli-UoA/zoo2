import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

import type { UserRow } from "../../../lib/electric/types";
import { MfmChannelsProvider } from "./MfmChannels";
import { storyChannels } from "../../../../.storybook/fixtures";
import { storyCommands } from "../../../../.storybook/app-fixtures";
import { Composer } from "./Composer";

const mentionUsers: UserRow[] = [
  {
    id: "user-alice",
    user_name: "alice",
    display_name: "Alice",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
  {
    id: "user-bob",
    user_name: "bob",
    display_name: "Bob",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
];

const meta = {
  title: "Features/Chat/Composer",
  component: Composer,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    disabled: { control: "boolean" },
    placeholder: { control: "text" },
    ariaLabel: { control: "text" },
  },
  args: {
    onTyping: fn(),
    onSend: fn(async () => true),
    commands: storyCommands,
    mentionUsers,
    mentionMemberIds: new Set(["user-alice", "user-bob"]),
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-3xl bg-background">
        <MfmChannelsProvider channels={storyChannels}>
          <Story />
        </MfmChannelsProvider>
      </div>
    ),
  ],
} satisfies Meta<typeof Composer>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Disabled: Story = {
  args: {
    disabled: true,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("textbox")).toBeDisabled();
  },
};

export const Multiline: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const textarea = canvas.getByRole("textbox");
    await userEvent.type(textarea, "1行目{enter}2行目{enter}3行目");
    await expect(textarea).toHaveValue("1行目\n2行目\n3行目");
  },
};

export const MentionSuggestions: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.type(canvas.getByRole("textbox"), "@al");
    await expect(canvas.getByRole("listbox", { name: "メンション候補" })).toBeVisible();
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    placeholder: "モバイルからメッセージを送信",
  },
};

export const ChannelSuggestions: Story = {
  parameters: { a11y: { test: "error" } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.type(canvas.getByRole("textbox"), "詳しくは #gen");
    await expect(canvas.getByRole("listbox", { name: "チャンネル候補" })).toBeVisible();
    await expect(canvas.getByRole("option", { name: "#general" })).toBeVisible();
  },
};
export const CommandSuggestions: Story = {
  parameters: { a11y: { test: "error" } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    const input = canvas.getByRole("textbox");
    await userEvent.type(input, "/ec");
    await expect(canvas.getByRole("listbox", { name: "コマンド候補" })).toBeVisible();
    await userEvent.click(canvas.getByRole("option", { name: "/echo Meeting Notes" }));
    await expect(input).toHaveValue("/echo ");
    // Wait for Composer to restore the caret before typing the command argument.
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await userEvent.type(input, "hello");
    await userEvent.click(canvas.getByRole("button", { name: "送信" }));
    await expect(args.onSend).toHaveBeenCalledWith("/echo hello", "notes");
  },
};
function pasteImage(target: HTMLElement) {
  const clipboardData = new DataTransfer();
  clipboardData.items.add(
    new File([new Uint8Array([1, 2, 3, 4])], "screenshot.png", { type: "image/png" }),
  );
  target.dispatchEvent(
    new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }),
  );
}

export const WithAttachments: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    onUploadAttachment: fn(async () => ({
      id: "attachment-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    })),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    pasteImage(canvas.getByRole("textbox"));
    await expect(await canvas.findByRole("img", { name: "screenshot.png" })).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: "送信" }));
    await expect(args.onSend).toHaveBeenCalledWith("", undefined, ["attachment-1"]);
  },
};

export const AttachmentUploadError: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    onUploadAttachment: fn(async () => {
      throw new Error("upload failed");
    }),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    pasteImage(canvas.getByRole("textbox"));
    await expect(await canvas.findByRole("alert")).toHaveTextContent(
      "画像をアップロードできませんでした。",
    );
    await expect(canvas.queryByRole("img", { name: "screenshot.png" })).toBeNull();
  },
};

export const ChannelInsertion: Story = {
  parameters: { a11y: { test: "error" } },
  globals: { viewport: { value: "mobile" } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const input = canvas.getByRole("textbox");
    await userEvent.type(input, "#gen");
    await userEvent.keyboard("{Tab}");
    await expect(input).toHaveValue("#general ");
    await expect(canvas.queryByRole("listbox")).not.toBeInTheDocument();
  },
};
