import type { Meta, StoryObj } from "@storybook/react-vite";
import { ScrollArea } from "./scroll-area";
import { Separator } from "./separator";
const meta = {
  title: "Components/ScrollArea",
  component: ScrollArea,
  parameters: { layout: "centered", a11y: { test: "error" } },
  args: {
    className: "h-64 w-72 rounded-lg border p-3",
    children: (
      <div>
        {Array.from({ length: 30 }, (_, i) => (
          <div key={i}>
            <p className="py-3">Channel {i + 1}</p>
            <Separator />
          </div>
        ))}
      </div>
    ),
  },
} satisfies Meta<typeof ScrollArea>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Overflow: Story = {};
export const ShortContent: Story = { args: { children: <p>General</p> } };
