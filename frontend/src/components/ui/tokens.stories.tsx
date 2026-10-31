import type { Meta, StoryObj } from "@storybook/react-vite";
const pairs = [
  ["background", "foreground"],
  ["card", "card-foreground"],
  ["popover", "popover-foreground"],
  ["primary", "primary-foreground"],
  ["secondary", "secondary-foreground"],
  ["muted", "muted-foreground"],
  ["accent", "accent-foreground"],
  ["brand-muted", "brand-muted-foreground"],
];
function Tokens() {
  return (
    <main className="space-y-6 bg-background p-6 text-foreground">
      <h1 className="text-2xl font-semibold">Design tokens</h1>
      <div className="grid gap-3 sm:grid-cols-2">
        {pairs.map(([background, foreground]) => (
          <div
            key={background}
            className="rounded-lg border p-4"
            style={{ background: `var(--${background})`, color: `var(--${foreground})` }}
          >
            <p className="font-medium">{background}</p>
            <p className="text-sm">{foreground} · Aa あいう 中文 0123</p>
          </div>
        ))}
      </div>
      <section className="space-y-3">
        <h2 className="text-lg font-semibold">Typography</h2>
        {["text-xs", "text-sm", "text-base", "text-lg", "text-2xl"].map((size) => (
          <p key={size} className={size}>
            {size} · Zooで会話を始める Aa 中文 0123
          </p>
        ))}
      </section>
      <section className="space-y-3">
        <h2 className="text-lg font-semibold">Spacing</h2>
        {[1, 2, 3, 4, 6, 8].map((step) => (
          <div key={step} className="flex items-center gap-3">
            <code>{step} × --spacing</code>
            <div className="h-4 bg-primary" style={{ width: `calc(var(--spacing) * ${step})` }} />
          </div>
        ))}
      </section>
    </main>
  );
}
const meta = {
  title: "Foundations/Tokens",
  component: Tokens,
  parameters: { a11y: { test: "error" } },
} satisfies Meta<typeof Tokens>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
