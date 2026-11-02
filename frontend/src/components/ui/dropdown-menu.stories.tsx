import { dropdownExample } from "../../../.storybook/primitive-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  ArchiveIcon,
  MoreHorizontalIcon,
  PencilIcon,
  ShieldAlertIcon,
  TrashIcon,
} from "lucide-react";
import { expect, within } from "storybook/test";
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
          {dropdownExample.trigger}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="center">
          <DropdownMenuItem>
            <PencilIcon />
            {dropdownExample.items[0].label}
          </DropdownMenuItem>
          <DropdownMenuItem>
            <ArchiveIcon />
            {dropdownExample.items[1].label}
          </DropdownMenuItem>
          <DropdownMenuItem>
            <ShieldAlertIcon />
            {dropdownExample.items[2].label}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive">
            <TrashIcon />
            {dropdownExample.items[3].label}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

function IconOnlyDropdownDemo() {
  return (
    <div className="flex min-h-72 items-start justify-center bg-background p-12">
      <DropdownMenu defaultOpen>
        <DropdownMenuTrigger
          render={<Button variant="ghost" size="icon-sm" aria-label="メニュー" />}
        >
          <MoreHorizontalIcon />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="center">
          <DropdownMenuItem>
            <PencilIcon />
            {dropdownExample.items[0].label}
          </DropdownMenuItem>
          <DropdownMenuItem>
            <ArchiveIcon />
            {dropdownExample.items[1].label}
          </DropdownMenuItem>
          <DropdownMenuItem>
            <ShieldAlertIcon />
            {dropdownExample.items[2].label}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive">
            <TrashIcon />
            {dropdownExample.items[3].label}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

const meta = {
  title: "Components/DropdownMenu",
  component: DropdownMenu,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  render: () => <DropdownDemo />,
} satisfies Meta<typeof DropdownMenu>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Open: Story = {
  render: () => <DropdownDemo defaultOpen />,
  play: playForImplementation(async ({ canvasElement }) => {
    const portal = within(canvasElement.ownerDocument.body);
    await expect(
      await portal.findByRole("menuitem", { name: "名前を変更" }, { timeout: 3000 }),
    ).toBeInTheDocument();
  }),
};

export const IconOnlyTrigger: Story = {
  render: () => <IconOnlyDropdownDemo />,
  play: playForImplementation(async ({ canvasElement }) => {
    const portal = within(canvasElement.ownerDocument.body);
    const menu = await portal.findByRole("menu");
    const item = within(menu).getByRole("menuitem", { name: "チャンネル設定" });
    await expect(getComputedStyle(item).whiteSpace).toBe("nowrap");
    await expect(menu.offsetWidth).toBeGreaterThanOrEqual(176);
  }),
};
