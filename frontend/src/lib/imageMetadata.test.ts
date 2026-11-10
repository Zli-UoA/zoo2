import { expect, it } from "vite-plus/test";
import { imageAspectRatio } from "./imageMetadata";

it("uses valid image dimensions and bounds extreme or missing metadata", () => {
  expect(imageAspectRatio({ width: 600, height: 400 })).toBe(1.5);
  expect(imageAspectRatio({ width: 400, height: 600 })).toBe(2 / 3);
  expect(imageAspectRatio({ width: 0, height: -1 })).toBe(4 / 3);
  expect(imageAspectRatio({})).toBe(4 / 3);
  expect(imageAspectRatio({ width: 1, height: 100000 })).toBe(0.1);
  expect(imageAspectRatio({ width: Infinity, height: 1 })).toBe(4 / 3);
});
