import type { ReactRenderer } from "@storybook/react-vite";
import type { Args, PlayFunction } from "storybook/internal/types";

export function playForImplementation<TArgs = Args>(
  play: PlayFunction<ReactRenderer, TArgs>,
  afterPlay?: PlayFunction<ReactRenderer, TArgs>,
): PlayFunction<ReactRenderer, TArgs> {
  return async (context) => {
    await play(context);
    await afterPlay?.(context);
  };
}
