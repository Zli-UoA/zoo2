import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";
import { OfflineContext, type OfflineContextValue } from "../../lib/offline/offline-context";
import { OfflineBanner } from "./OfflineBanner";
const online: OfflineContextValue = {
  isOnline: true,
  persistenceStatus: "ready",
  persistenceError: null,
  namespace: "storybook",
  retryPersistence: fn(),
  clearUserData: async () => {},
};
const meta = {
  title: "Features/Status/Offline",
  component: OfflineBanner,
  parameters: { a11y: { test: "error" }, offline: online },
  decorators: [
    (Story, context) => (
      <OfflineContext.Provider value={context.parameters.offline}>
        <div className="min-h-48 bg-background">
          <Story />
        </div>
      </OfflineContext.Provider>
    ),
  ],
} satisfies Meta<typeof OfflineBanner>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Online: Story = {};
export const Offline: Story = { parameters: { offline: { ...online, isOnline: false } } };
export const PersistenceError: Story = {
  parameters: {
    offline: {
      ...online,
      persistenceStatus: "error",
      persistenceError: "保存容量が不足しています。ストレージ設定を確認してください。",
    },
  },
};
export const Mobile: Story = { ...PersistenceError, globals: { viewport: { value: "mobile" } } };
