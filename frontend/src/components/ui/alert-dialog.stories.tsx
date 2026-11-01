import { dialogSamples } from "../../../.storybook/dialog-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";
import { useState } from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "./alert-dialog";
import { Button } from "./button";

function AlertDialogDemo({ defaultOpen = false }: { defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="flex min-h-96 items-center justify-center bg-background p-6">
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogTrigger render={<Button variant="outline" />}>
          {dialogSamples.alert.trigger}
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{dialogSamples.alert.title}</AlertDialogTitle>
            <AlertDialogDescription>{dialogSamples.alert.description}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{dialogSamples.alert.cancel}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => setOpen(false)}>
              {dialogSamples.alert.confirm}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

const meta = {
  title: "Components/AlertDialog",
  component: AlertDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  render: () => <AlertDialogDemo />,
} satisfies Meta<typeof AlertDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Open: Story = {
  render: () => <AlertDialogDemo defaultOpen />,
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
  }),
};
