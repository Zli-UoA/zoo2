import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, within } from "storybook/test";
import { useState } from "react";

import { RECENT_EMOJIS_STORAGE_KEY } from "../../../lib/emoji/recents";
import { Button } from "../../ui/button";
import { ReactionPicker } from "./ReactionPicker";

const minimalKitchenCatalog = {
  emojis: [
    ["1f600", "😀", "grinning face"],
    ["1f431", "🐱", "cat face"],
  ],
  dates: ["20260101"],
  pairs: { "0_1": 0 },
};

function ReactionPickerDemo({ onSelect }: { onSelect: (emoji: string) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="flex min-h-96 items-center justify-center bg-background p-8">
      <Button onClick={() => setOpen(true)}>リアクションを追加</Button>
      <ReactionPicker open={open} onOpenChange={setOpen} onSelect={onSelect} />
    </div>
  );
}

const meta = {
  title: "Features/Chat/ReactionPicker",
  component: ReactionPicker,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    open: { control: "boolean" },
  },
  args: {
    open: true,
    onOpenChange: fn(),
    onSelect: fn(),
  },
  render: (args) => <ReactionPickerDemo onSelect={args.onSelect} />,
} satisfies Meta<typeof ReactionPicker>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: async () => {
    const dialog = await screen.findByRole("dialog", {}, { timeout: 5000 });
    // The first open fetches the emoji catalogue and mounts a virtual grid.
    // Wait for that grid rather than applying Testing Library's 1s default.
    const picker = within(dialog);
    const rows = await picker.findAllByRole("row", {}, { timeout: 5000 });
    await expect(rows.every((row) => Number(row.getAttribute("aria-rowindex")) >= 1)).toBe(true);
    const cells = await picker.findAllByRole("gridcell", {}, { timeout: 5000 });
    await expect(cells.every((cell) => Number(cell.getAttribute("aria-colindex")) >= 1)).toBe(true);
  },
};

export const Kitchen: Story = {
  play: async () => {
    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    await expect(
      await screen.findByRole("combobox", { name: "1つ目の絵文字" }, { timeout: 5000 }),
    ).toBeVisible();
  },
};

export const SearchEmpty: Story = {
  play: async () => {
    const search = screen.getByPlaceholderText("絵文字を検索");
    await userEvent.type(search, "zzzzzzzz");
    await expect(await screen.findByText("一致する絵文字がありません")).toBeVisible();
  },
};

export const Recents: Story = {
  decorators: [
    (Story) => {
      localStorage.setItem(RECENT_EMOJIS_STORAGE_KEY, JSON.stringify(["🎉", "👍"]));
      return <Story />;
    },
  ],
};

export const KitchenLoading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get(/emoji-kitchen.*\.json/, async () => {
          await delay(1500);
          return HttpResponse.json(minimalKitchenCatalog);
        }),
      ],
    },
  },
  play: async () => {
    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
