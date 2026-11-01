import { runNativeInteractions } from "./native-interactions";
import type { ReactRenderer } from "@storybook/react-vite";
import type { Args, PlayFunction } from "storybook/internal/types";
import { waitFor } from "storybook/test";

/** React play functions target React DOM; Kotlin interactions are verified against the real iframe. */
export function playForImplementation<TArgs = Args>(
  reactPlay: PlayFunction<ReactRenderer, TArgs>,
): PlayFunction<ReactRenderer, TArgs> {
  return async (context) => {
    if (context.globals.implementation !== "kotlin") return reactPlay(context);
    await waitFor(
      () => {
        const root = context.canvasElement;
        if (root.querySelector('[data-kotlin-story-status="unavailable"]'))
          throw new Error("Kotlin story is not implemented");
        if (!root.querySelector('[data-compose-catalog-state="ready"]'))
          throw new Error("Kotlin preview is not ready");
      },
      { timeout: 90_000 },
    );
    const frame = context.canvasElement.querySelector<HTMLIFrameElement>(
      'iframe[title="Kotlin component preview"]',
    );
    if (!frame) throw new Error("Kotlin frame is missing");
    await runNativeInteractions(
      frame,
      JSON.parse(frame.dataset.composeCatalogInteractions ?? "[]"),
      { ...context.args, fixtureControls: context.parameters.fixtureControls },
    );
  };
}
