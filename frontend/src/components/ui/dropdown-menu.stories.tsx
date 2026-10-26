import type { Meta, StoryObj } from "@storybook/react-vite";
import { ArchiveIcon, PencilIcon, ShieldAlertIcon, TrashIcon } from "lucide-react";
import { expect, screen, userEvent, within } from "storybook/test";
import { useState } from "react";

import { Button } from "./button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./dropdown-menu";

function DropdownDemo({ defaultOpen = false }: { defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="flex min-h-72 items-start justify-center bg-background p-12">
      <DropdownMenu open={open} onOpenChange={setOpen}>
        <DropdownMenuTrigger render={<Button variant="outline" />}>
          チャンネル操作
        </DropdownMenuTrigger>
        <DropdownMenuContent align="center">
          <DropdownMenuItem>
            <PencilIcon />
            名前を変更
          </DropdownMenuItem>
          <DropdownMenuItem>
            <ArchiveIcon />
            アーカイブ
          </DropdownMenuItem>
          <DropdownMenuItem>
            <ShieldAlertIcon />
            メッセージ管理
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive">
            <TrashIcon />
            削除
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

const meta = {
  title: "Foundations/DropdownMenu",
  component: DropdownMenu,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  render: () => <DropdownDemo />,
} satisfies Meta<typeof DropdownMenu>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "チャンネル操作" }));
    await expect(
      await screen.findByRole("menuitem", { name: "名前を変更" }, { timeout: 3000 }),
    ).toBeInTheDocument();
  },
};

export const Open: Story = {
  render: () => <DropdownDemo defaultOpen />,
};
