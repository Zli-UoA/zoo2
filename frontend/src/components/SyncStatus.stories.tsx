import type { Meta, StoryObj } from "@storybook/react-vite";

import { SyncStatus } from "./SyncStatus";

const meta = {
  title: "Layout/SyncStatus",
  component: SyncStatus,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  decorators: [
    (Story) => (
      <div className="w-fit bg-background p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SyncStatus>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Proxy: Story = {};

export const Direct: Story = {
  parameters: {
    collections: {
      config: {
        shapeMode: "direct",
        txidMode: "awaitMatch",
        devUserId: null,
      },
    },
  },
};

export const Oidc: Story = {
  parameters: {
    collections: {
      config: {
        devUserId: null,
        oidc: {
          issuer: "https://id.example",
          clientId: "zoo",
          redirectUri: null,
          scope: "openid profile",
          resource: null,
          postLogoutRedirectUri: null,
        },
      },
    },
  },
};
