import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import { TelemetryErrorBoundary } from "./TelemetryErrorBoundary";

function HealthyContent() {
  return <p className="p-4 text-sm text-foreground">表示は正常です。</p>;
}

function ThrowingContent(): never {
  throw new Error("Storybook simulated render failure");
}

const meta = {
  title: "Components/TelemetryErrorBoundary",
  component: TelemetryErrorBoundary,
  parameters: {
    layout: "centered",
    a11y: { test: "error" },
  },
} satisfies Meta<typeof TelemetryErrorBoundary>;

export default meta;
type Story = StoryObj<typeof meta>;

export const HealthyChild: Story = {
  args: {
    children: <HealthyContent />,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("表示は正常です。")).toBeVisible();
  },
};

export const FallbackShown: Story = {
  args: {
    children: <ThrowingContent />,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("button")).toBeVisible();
  },
};
