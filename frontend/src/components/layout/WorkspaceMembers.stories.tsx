import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent } from "storybook/test";
import { storyCollections } from "../../../.storybook/fixtures";
import { WorkspaceMembers } from "./WorkspaceMembers";
const adminIdentity = http.get("*/api/users/me", () =>
  HttpResponse.json({
    id: "user-alice",
    user_name: "alice",
    display_name: "Alice",
    is_admin: true,
  }),
);
const meta = {
  title: "Features/Workspace/Members",
  component: WorkspaceMembers,
  parameters: { a11y: { test: "error" } },
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
export const ReadOnly: Story = {};
export const Admin: Story = {
  parameters: { msw: { handlers: [adminIdentity] } },
  play: async () => {
    await expect(await screen.findAllByRole("button", { name: "管理者にする" })).not.toHaveLength(
      0,
    );
  },
};
export const Empty: Story = { parameters: { collections: { ...storyCollections, users: [] } } };
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/users/admins", async () => {
          await delay("infinite");
          return HttpResponse.json({ user_ids: [] });
        }),
      ],
    },
  },
};
export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [http.get("*/api/users/admins", () => new HttpResponse(null, { status: 500 }))],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  },
};
export const SearchEmpty: Story = {
  play: async () => {
    await userEvent.type(screen.getByRole("searchbox"), "no-such-member");
    await expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
  },
};
export const Mobile: Story = { ...Admin, globals: { viewport: { value: "mobile" } } };

export const ProtectedAdmin: Story = {
  parameters: {
    msw: {
      handlers: [
        adminIdentity,
        http.get("*/api/users/admins", () =>
          HttpResponse.json({
            user_ids: ["user-alice", "user-admin"],
            protected_user_ids: ["user-admin"],
          }),
        ),
      ],
    },
  },
};
export const LastAdmin: Story = {
  parameters: {
    msw: {
      handlers: [
        adminIdentity,
        http.get("*/api/users/admins", () =>
          HttpResponse.json({ user_ids: ["user-alice"], protected_user_ids: [] }),
        ),
      ],
    },
  },
};
