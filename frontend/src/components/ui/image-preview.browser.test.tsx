import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { ImagePreview } from "./image-preview";
import { imageAspectRatio } from "../../lib/imageMetadata";
import "../../styles.css";

it.each([160, 640])("keeps layout stable and paints a placeholder at %ipx", async (width) => {
  const metadata = { width: 600, height: 400, blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj" };
  const element = (src?: string) => (
    <div style={{ width }}>
      <ImagePreview
        {...metadata}
        src={src}
        alt="Photo"
        style={{ aspectRatio: imageAspectRatio(metadata) }}
      />
      <div data-testid="next">Next post</div>
    </div>
  );
  const view = render(element());
  const top = screen.getByTestId("next").getBoundingClientRect().top;
  const canvas = view.container.querySelector("canvas")!;
  expect(canvas.getContext("2d")!.getImageData(0, 0, 1, 1).data[3]).toBe(255);
  const src =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=";
  view.rerender(element(src));
  const img = screen.getByRole("img", { name: "Photo" });
  await waitFor(() => expect(img.className).toContain("opacity-100"));
  expect(view.container.querySelector("canvas")).toBeNull();
  expect(screen.getByTestId("next").getBoundingClientRect().top).toBe(top);
  view.rerender(element("/image-pending.png"));
  fireEvent.error(screen.getByRole("img", { name: "Photo" }));
  expect(screen.getByTestId("next").getBoundingClientRect().top).toBe(top);
});

it("ignores invalid optional metadata", () => {
  const view = render(
    <ImagePreview
      alt="Old image"
      blurhash="bad"
      style={{ aspectRatio: imageAspectRatio({ width: -1, height: 0 }) }}
    />,
  );
  expect(view.container.querySelector("canvas")).not.toBeNull();
  expect(imageAspectRatio({ width: -1, height: 0 })).toBe(4 / 3);
});
