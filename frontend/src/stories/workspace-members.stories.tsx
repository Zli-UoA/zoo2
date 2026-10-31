import { playForImplementation } from "../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import { WorkspaceMembers } from "../components/layout/WorkspaceMembers";
const meta = {
  title: "Flows/Workspace/ChangeRole",
  component: WorkspaceMembers,
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/users/me", () =>
          HttpResponse.json({
            id: "user-alice",
            user_name: "alice",
            display_name: "Alice",
            is_admin: true,
          }),
        ),
        http.get("*/api/users/admins", () =>
          HttpResponse.json({
            user_ids: ["user-alice", "user-admin"],
            protected_user_ids: ["user-admin"],
          }),
        ),
        http.patch("*/api/users/:id/admin", () => HttpResponse.json({})),
      ],
    },
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof WorkspaceMembers>;
export default meta;
type Story = StoryObj<typeof meta>;
export const GrantAndRevoke: Story = {
  play: playForImplementation(async () => {
    const bob = (await screen.findByText("@bob")).closest("li")!;
    const makeAdmin = await within(bob).findByRole("button", { name: "管理者にする" });
    await waitFor(() => expect(makeAdmin).toBeEnabled());
    await userEvent.click(makeAdmin);
    const grant = await screen.findByRole("alertdialog");
    await userEvent.click(within(grant).getByRole("button", { name: "管理者にする" }));
    const revoke = await within(bob).findByRole("button", { name: "管理者を外す" });
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    await waitFor(() => expect(revoke).toBeEnabled());
    await userEvent.click(revoke);
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "管理者を外す" }),
    );
    await expect(await within(bob).findByRole("button", { name: "管理者にする" })).toBeEnabled();
  }),
};
export const RejectedLastAdmin: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/users/me", () =>
          HttpResponse.json({
            id: "user-alice",
            user_name: "alice",
            display_name: "Alice",
            is_admin: true,
          }),
        ),
        // Another administrator was removed after this list was fetched.
        http.get("*/api/users/admins", () =>
          HttpResponse.json({ user_ids: ["user-alice", "user-bob"] }),
        ),
        http.patch("*/api/users/:id/admin", () => new HttpResponse(null, { status: 409 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    const bob = (await screen.findByText("@bob")).closest("li")!;
    const makeAdmin = await within(bob).findByRole("button", { name: "管理者を外す" });
    await waitFor(() => expect(makeAdmin).toBeEnabled());
    await userEvent.click(makeAdmin);
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "管理者を外す" }),
    );
    await waitFor(() => expect(screen.getByRole("alert")).toBeVisible());
  }),
};
