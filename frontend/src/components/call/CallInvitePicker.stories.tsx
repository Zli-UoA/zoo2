import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { CallInvitePicker } from "./CallInvitePicker";

const candidates = [
  { id: "user-bob", name: "Bob" },
  { id: "user-carol", name: "Carol" },
  { id: "user-dave", name: "Dave" },
];

const meta = {
  title: "Features/Calls/InvitePicker",
  component: CallInvitePicker,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: { candidates, slots: 3, onInvite: fn() },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="w-80 rounded-xl border bg-popover p-4 text-popover-foreground">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof CallInvitePicker>;

export default meta;
type Story = StoryObj<typeof meta>;

/** メンバーを選んで呼ぶ。選んだ人の ID だけが渡る。 */
export const SelectAndInvite: Story = {
  play: playForImplementation(async ({ args }) => {
    const submit = await screen.findByRole("button", { name: /呼び出す/ });
    await expect(submit).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /Bob/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: /Dave/ }));
    await userEvent.click(screen.getByRole("button", { name: "2 人を呼び出す" }));
    await expect(args.onInvite).toHaveBeenCalledWith(["user-bob", "user-dave"]);
  }),
};

/** 空きの数までしか選べない。 */
export const LimitedSlots: Story = {
  args: { slots: 1 },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("checkbox", { name: /Bob/ }));
    await expect(screen.getByRole("checkbox", { name: /Carol/ })).toBeDisabled();
  }),
};

export const Full: Story = {
  args: { slots: 0 },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("通話の定員に達しています")).toBeInTheDocument();
  }),
};

export const Nobody: Story = {
  args: { candidates: [] },
};

export const Failed: Story = {
  args: { error: "呼び出せませんでした" },
};

export const Sending: Story = {
  args: { busy: true },
};
