import type { ReactNode } from "react";
import { render as renderComponent, screen, fireEvent, waitFor } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import pdfUrl from "../../../.storybook/fixtures/media.pdf?url";
import { MediaPreview } from "./media-preview";
import "../../styles.css";
import { LocaleProvider } from "../i18n/LocaleProvider";

const render = (element: ReactNode) => renderComponent(<LocaleProvider>{element}</LocaleProvider>);

it.each([390, 1280])("defers PDF loading and fits the viewer at %ipx", async (width) => {
  await page.viewport(width, 844);
  const view = render(
    <div style={{ width: 280 }}>
      <MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" />
    </div>,
  );
  expect(view.container.querySelector("canvas")).toBeNull();
  fireEvent.click(screen.getByRole("button"));
  const dialog = await screen.findByRole("dialog");
  const canvas = dialog.querySelector("canvas")!;
  await waitFor(() => expect(canvas.width).toBeGreaterThan(300), { timeout: 10_000 });
  expect(canvas.getBoundingClientRect().width).toBeGreaterThan(280);
  const next = screen.getByRole("button", { name: "次のページ" });
  await waitFor(() => expect(next).not.toBeDisabled(), { timeout: 10_000 });
  fireEvent.click(next);
  await screen.findByText("2 / 12");
});

it.each(["video/mp4", "audio/mpeg"])("provides controls without autoplay for %s", (contentType) => {
  const view = render(<MediaPreview src="/media" name="Media" contentType={contentType} />);
  const media = view.container.querySelector("video, audio") as HTMLMediaElement;
  expect(media.controls).toBe(true);
  expect(media.autoplay).toBe(false);
  expect(media.preload).toBe("none");
  fireEvent.error(media);
  expect(screen.getByRole("alert")).toBeTruthy();
});

it("keeps full width in a narrow placement and opens the same image viewer", async () => {
  const view = render(
    <div style={{ width: 280 }}>
      <MediaPreview
        src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII="
        name="Photo"
        contentType="image/png"
      />
    </div>,
  );
  expect(view.container.querySelector("button")!.getBoundingClientRect().width).toBeGreaterThan(
    270,
  );
  fireEvent.click(screen.getByRole("button"));
  expect(await screen.findByRole("dialog")).toBeTruthy();
});
