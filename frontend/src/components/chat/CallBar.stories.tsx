import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { storyChannels } from "../../../.storybook/fixtures";
import { CallContext } from "../../lib/call/call-context";
import type { CallContextValue, CallParticipant } from "../../lib/call/types";
import { CallBar } from "./CallBar";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;

const mockParticipants: CallParticipant[] = [
  {
    session_id: "session-1",
    user_id: "user-1",
    user_name: "alice",
    display_name: "Alice",
    is_muted: false,
    joined_at: "2026-01-01T00:00:00Z",
  },
  {
    session_id: "session-2",
    user_id: "user-2",
    user_name: "bob",
    display_name: "Bob",
    is_muted: true,
    joined_at: "2026-01-01T00:00:00Z",
  },
];

function createMockContext(overrides: Partial<CallContextValue> = {}): CallContextValue {
  return {
    currentChannelId: general.id,
    connectionState: "connected",
    error: null,
    participants: mockParticipants,
    isMuted: false,
    speakingUserIds: new Set(["user-1"]),
    activeCalls: new Map([[general.id, 2]]),
    joinCall: async () => {},
    leaveCall: async () => {},
    toggleMute: async () => {},
    ...overrides,
  };
}

const meta = {
  title: "Features/Chat/CallBar",
  component: CallBar,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    channels: storyChannels,
  },
} satisfies Meta<typeof CallBar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  decorators: [
    (Story) => (
      <CallContext.Provider value={createMockContext()}>
        <div className="relative h-40 w-full bg-background p-4">
          <Story />
        </div>
      </CallContext.Provider>
    ),
  ],
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(general.channel_name)).toBeInTheDocument();
  }),
};

export const Connecting: Story = {
  decorators: [
    (Story) => (
      <CallContext.Provider
        value={createMockContext({
          connectionState: "connecting",
          participants: [],
          speakingUserIds: new Set(),
        })}
      >
        <div className="relative h-40 w-full bg-background p-4">
          <Story />
        </div>
      </CallContext.Provider>
    ),
  ],
};

export const Muted: Story = {
  decorators: [
    (Story) => (
      <CallContext.Provider
        value={createMockContext({
          isMuted: true,
        })}
      >
        <div className="relative h-40 w-full bg-background p-4">
          <Story />
        </div>
      </CallContext.Provider>
    ),
  ],
};

export const Speaking: Story = {
  decorators: [
    (Story) => (
      <CallContext.Provider
        value={createMockContext({
          speakingUserIds: new Set(["user-1", "user-2"]),
        })}
      >
        <div className="relative h-40 w-full bg-background p-4">
          <Story />
        </div>
      </CallContext.Provider>
    ),
  ],
};

export const MultipleParticipants: Story = {
  decorators: [
    (Story) => (
      <CallContext.Provider
        value={createMockContext({
          participants: [
            ...mockParticipants,
            {
              session_id: "session-3",
              user_id: "user-3",
              user_name: "carol",
              display_name: "Carol",
              is_muted: false,
              joined_at: "2026-01-01T00:00:00Z",
            },
            {
              session_id: "session-4",
              user_id: "user-4",
              user_name: "dave",
              display_name: "Dave",
              is_muted: true,
              joined_at: "2026-01-01T00:00:00Z",
            },
            {
              session_id: "session-5",
              user_id: "user-5",
              user_name: "eve",
              display_name: "Eve",
              is_muted: false,
              joined_at: "2026-01-01T00:00:00Z",
            },
          ],
        })}
      >
        <div className="relative h-40 w-full bg-background p-4">
          <Story />
        </div>
      </CallContext.Provider>
    ),
  ],
};
