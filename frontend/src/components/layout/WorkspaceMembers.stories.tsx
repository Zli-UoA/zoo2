import { preferenceSurfaces } from "../../../.storybook/preference-stories";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
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
      <div className={preferenceSurfaces["Features/Workspace/Members"]}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof WorkspaceMembers>;
export default meta;
type Story = StoryObj<typeof meta>;
export const ReadOnly: Story = {
  play: playForImplementation(async () => {
    await screen.findByText("@admin");
    await screen.findByText("管理者");
    await expect(screen.getAllByRole("listitem")).toHaveLength(storyCollections.users.length);
    await expect(screen.queryByRole("button", { name: "管理者にする" })).not.toBeInTheDocument();
    await expect(screen.queryByRole("button", { name: "管理者を外す" })).not.toBeInTheDocument();
  }),
};
export const Admin: Story = {
  parameters: { msw: { handlers: [adminIdentity] } },
  play: playForImplementation(async () => {
    await expect(await screen.findAllByRole("button", { name: "管理者にする" })).not.toHaveLength(
      0,
    );
  }),
};
export const Empty: Story = {
  parameters: { collections: { ...storyCollections, users: [] } },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("該当するユーザーがいません。")).toBeVisible();
    await expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
  }),
};
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
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  }),
};
export const SearchEmpty: Story = {
  play: playForImplementation(async () => {
    await userEvent.type(await screen.findByRole("searchbox"), "no-such-member");
    await expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
  }),
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
  play: playForImplementation(async () => {
    const protectedRow = (await screen.findByText("@admin")).closest("li")!;
    const alice = (await screen.findByText("@alice")).closest("li")!;
    await waitFor(() =>
      expect(within(alice).getByRole("button", { name: "管理者を外す" })).toBeEnabled(),
    );
    await expect(within(protectedRow).getByRole("button", { name: "管理者を外す" })).toBeDisabled();
  }),
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
  play: playForImplementation(async () => {
    const alice = (await screen.findByText("@alice")).closest("li")!;
    const other = (await screen.findByText("@admin")).closest("li")!;
    await waitFor(() =>
      expect(within(other).getByRole("button", { name: "管理者にする" })).toBeEnabled(),
    );
    await expect(within(alice).getByRole("button", { name: "管理者を外す" })).toBeDisabled();
  }),
};
