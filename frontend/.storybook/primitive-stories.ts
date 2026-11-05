import { dropdownExample, tooltipExample, toastExample, tokenPairs } from "./primitive-fixtures";
import type { KotlinPreviewProps } from "./KotlinPreview";
import type { Selection } from "./kotlin-stories";

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : {};
}
function text(value: unknown): string {
  if (typeof value === "string" || typeof value === "number") return String(value);
  if (Array.isArray(value)) return value.map(text).join("");
  return text(record(record(value).props).children ?? "");
}
const titles = new Set([
  "Foundations/Tokens",
  "Components/ScrollArea",
  "Components/CollapsibleSection",
  "Components/DropdownMenu",
  "Components/Tooltip",
  "Components/Toaster",
]);
export function primitiveOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  if (!titles.has(context.title)) return null;
  const args = context.args;
  const interactions: Record<string, unknown>[] = [];
  if (context.title === "Components/DropdownMenu" && context.name === "Open")
    interactions.push({
      action: "wait",
      role: "menuitem",
      name: dropdownExample.items[0].label,
      exact: true,
    });
  if (context.title === "Components/DropdownMenu" && context.name === "Icon Only Trigger")
    interactions.push(
      { action: "wait", role: "menu", minWidth: 176 },
      {
        action: "wait",
        role: "menuitem",
        name: dropdownExample.items[2].label,
        exact: true,
        maxHeight: 28,
      },
    );
  if (context.title === "Components/Tooltip" && context.name === "Open")
    interactions.push(
      {
        action: "hover",
        role: "button",
        name: tooltipExample.trigger,
        exact: true,
      },
      { action: "wait", role: "tooltip" },
      { action: "wait", text: tooltipExample.content, within: { role: "tooltip" }, exact: true },
    );
  if (context.title === "Components/Toaster" && context.name === "Success")
    interactions.push(
      { action: "click", role: "button", name: toastExample.trigger, exact: true },
      { action: "wait", text: toastExample.content, exact: true },
    );
  if (context.title === "Components/CollapsibleSection" && context.name === "With Actions")
    interactions.push(
      { action: "click", role: "button", name: "チャンネルを作成", exact: true },
      { action: "wait", text: text(args.children), state: "visible", exact: true },
      { action: "click", role: "button", name: args.title ?? "チャンネル", exact: true },
      { action: "wait", text: text(args.children), state: "hidden", exact: true },
      { action: "key", key: "Enter" },
      { action: "wait", text: text(args.children), state: "visible", exact: true },
      { action: "key", key: "Space" },
      { action: "wait", text: text(args.children), state: "hidden", exact: true },
    );
  return {
    ...base,
    sample: "primitive-story",
    fullHeight: context.title !== "Foundations/Tokens",
    viewportHeightInset: context.title === "Components/ScrollArea" ? 32 : 0,
    surfaceClassName: undefined,
    fixture: {
      ...base.fixture,
      primitive: context.title,
      dropdown: dropdownExample,
      tooltip: tooltipExample,
      toast: toastExample,
      tokenPairs,
      interactions,
      state: context.name,
      title: args.title ?? "チャンネル",
      count:
        typeof args.count === "number" || typeof args.count === "string"
          ? args.count
          : args.count
            ? text(args.count)
            : null,
      content: text(args.children),
      scrollRows: Array.isArray(record(record(args.children).props).children)
        ? (record(record(args.children).props).children as unknown[]).map(text)
        : [],
      defaultOpen: args.defaultOpen !== false,
      hasActions: Boolean(args.actions),
      className: typeof args.className === "string" ? args.className : "",
    },
  };
}
