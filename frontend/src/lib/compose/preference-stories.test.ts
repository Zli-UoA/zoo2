import { describe, expect, it } from "vite-plus/test";
import { kotlinOptions, type Selection } from "../../../.storybook/kotlin-stories";
import type { NativeInteraction } from "../../../.storybook/native-interactions";

function steps(title: string, name: string): NativeInteraction[] {
  const context: Selection = {
    id: "preference-contract--state",
    title,
    name,
    args: {},
    parameters: {},
    globals: { implementation: "kotlin", theme: "light", locale: "ja" },
  };
  return kotlinOptions(context)!.fixture!.interactions as NativeInteraction[];
}

describe("preference Storybook interaction contracts", () => {
  it("queries loading and failure controls in their own feature", () => {
    const searchLoading = steps("Features/Preferences/MfmSearch", "Loading");
    expect(searchLoading).toContainEqual(
      expect.objectContaining({
        action: "wait",
        role: "combobox",
        name: "検索エンジン",
        enabled: false,
        busy: true,
      }),
    );
    expect(steps("Features/Workspace/Members", "Loading")).toEqual([]);
    const membersFailure = steps("Features/Workspace/Members", "Load Error");
    expect(membersFailure.some((step) => step.role === "alert")).toBe(true);
    expect(membersFailure.some((step) => step.role === "combobox")).toBe(false);
  });

  it("verifies the default theme's three controls and its single selected value", () => {
    const selection = steps("Features/Preferences/Theme", "Default");
    expect(selection).toContainEqual(expect.objectContaining({ role: "button", count: 3 }));
    const buttons = selection.filter(
      (step) => step.role === "button" && step.pressed !== undefined,
    );
    expect(buttons).toHaveLength(3);
    expect(buttons.filter((step) => step.pressed).map((step) => step.name)).toEqual(["ライト"]);
  });
  it("opens the ActivityPub tab for profile and moved channel-rule states", () => {
    for (const name of [
      "Activity Pub",
      "Activity Pub Mobile",
      "Activity Pub Loading",
      "Activity Pub Error",
      "Activity Pub Save Error",
      "Channel Rule",
      "Channel Rule Error",
    ]) {
      const actions = steps("Pages/ZooSettings", name);
      expect(actions[0]).toEqual(
        expect.objectContaining({ action: "click", role: "tab", name: "ActivityPub" }),
      );
      expect(actions.some((step) => step.role === "combobox" && step.name === "言語")).toBe(false);
      for (const step of actions.filter(
        (step) => step.action === "click" && step.name === "保存",
      )) {
        expect(step.within).toEqual(expect.objectContaining({ role: "form" }));
      }
    }
  });
});
