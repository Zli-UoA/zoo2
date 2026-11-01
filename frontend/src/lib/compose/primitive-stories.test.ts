import { describe, expect, it } from "vite-plus/test";
import { primitiveOptions } from "../../../.storybook/primitive-stories";
import type { KotlinPreviewProps } from "../../../.storybook/KotlinPreview";
import type { Selection } from "../../../.storybook/kotlin-stories";
const base: KotlinPreviewProps = {
  sample: "message",
  content: "",
  disabled: false,
  grouped: false,
  dark: false,
  locale: "ja",
  fixture: { sharedApi: true },
};
function context(title: string, name: string, args: Record<string, unknown> = {}): Selection {
  return { id: "example", title, name, args, parameters: {}, globals: {} };
}
describe("primitive story adaptation", () => {
  it("keeps nested scroll content from the current Controls rather than substituting demo rows", () => {
    const child = (value: string) => ({
      type: "div",
      props: {
        children: [
          { type: "p", props: { children: value } },
          { type: "hr", props: {} },
        ],
      },
    });
    const options = primitiveOptions(
      context("Components/ScrollArea", "Overflow", {
        children: { type: "div", props: { children: [child("Changed channel"), child("Second")] } },
      }),
      base,
    )!;
    expect(options.fixture?.scrollRows).toEqual(["Changed channel", "Second"]);
    expect(options.fixture?.sharedApi).toBe(true);
  });
  it("preserves disclosure state while omitting nonserializable React action objects", () => {
    const options = primitiveOptions(
      context("Components/CollapsibleSection", "Closed", {
        title: "DM",
        count: 0,
        defaultOpen: false,
        actions: { type: () => null, props: { onClick: () => undefined } },
        children: { type: "div", props: { children: "Same content" } },
      }),
      base,
    )!;
    expect(options.fixture).toMatchObject({
      title: "DM",
      count: 0,
      defaultOpen: false,
      hasActions: true,
      content: "Same content",
    });
    expect(() => structuredClone(options)).not.toThrow();
  });
  it.each([
    ["Components/DropdownMenu", "Open", "menuitem"],
    ["Components/DropdownMenu", "Icon Only Trigger", "menuitem"],
    ["Components/Tooltip", "Open", "tooltip"],
    ["Components/Toaster", "Success", undefined],
  ])("verifies the visible result of %s/%s", (title, name, role) => {
    const options = primitiveOptions(context(title!, name!), base)!;
    const steps = options.fixture?.interactions as { action: string; role?: string }[];
    expect(steps.some((step) => step.action === "wait" && step.role === role)).toBe(true);
  });
  it("rejects unrelated components instead of using a generic primitive fallback", () => {
    expect(primitiveOptions(context("Pages/Channel", "Default"), base)).toBeNull();
  });
});
