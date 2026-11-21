import type { ReactNode } from "react";
import { render as renderComponent, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import pdfUrl from "../../../.storybook/fixtures/media.pdf?url";
import { MediaPreview } from "./media-preview";
import "../../styles.css";
import { LocaleProvider } from "../i18n/LocaleProvider";

const render = (element: ReactNode) => renderComponent(<LocaleProvider>{element}</LocaleProvider>);

it.each([390, 1280])(
  "defers PDF loading and supports vertical scroll, page buttons, and page jump at %ipx",
  async (width) => {
    await page.viewport(width, 844);
    const user = userEvent.setup();
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

    const scrollRegion = screen.getByRole("region", { name: "PDFページ一覧" });
    const pageInput = screen.getByRole("textbox", { name: "ページ番号" }) as HTMLInputElement;
    const prev = screen.getByRole("button", { name: "前のページ" });
    const next = screen.getByRole("button", { name: "次のページ" });
    await waitFor(() => expect(next).not.toBeDisabled(), { timeout: 10_000 });
    expect(prev).toBeDisabled();
    expect(pageInput.value).toBe("1");
    expect(dialog.textContent).toContain("/ 12");

    // Pages are stacked vertically in continuous order.
    const slot1 = dialog.querySelector('[data-pdf-page="1"]')!;
    const slot2 = dialog.querySelector('[data-pdf-page="2"]')!;
    expect(slot2.getBoundingClientRect().top).toBeGreaterThan(slot1.getBoundingClientRect().bottom);

    // Previous / Next buttons step across pages.
    await user.click(next);
    await waitFor(() => expect(pageInput.value).toBe("2"), { timeout: 10_000 });
    await waitFor(
      () =>
        expect(
          (dialog.querySelector('[data-pdf-page="2"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(300),
      { timeout: 10_000 },
    );

    await user.click(prev);
    await waitFor(() => expect(pageInput.value).toBe("1"), { timeout: 10_000 });

    // Vertical scroll switches the current page and renders newly visible pages.
    const slot3 = dialog.querySelector('[data-pdf-page="3"]') as HTMLElement;
    scrollRegion.scrollTop = slot3.offsetTop;
    fireEvent.scroll(scrollRegion);
    await waitFor(() => expect(pageInput.value).toBe("3"), { timeout: 10_000 });
    await waitFor(
      () =>
        expect(
          (dialog.querySelector('[data-pdf-page="3"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(300),
      { timeout: 10_000 },
    );

    // Typing a page number and pressing Enter jumps directly and releases far off-screen canvases.
    await user.click(pageInput);
    await user.keyboard("10{Enter}");
    await waitFor(() => expect(pageInput.value).toBe("10"), { timeout: 10_000 });
    await waitFor(
      () =>
        expect(
          (dialog.querySelector('[data-pdf-page="10"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(300),
      { timeout: 10_000 },
    );
    expect(dialog.querySelector('[data-pdf-page="1"] canvas')).toBeNull();

    // Out-of-range numbers clamp cleanly and Previous page still steps back from the bottom.
    await user.click(pageInput);
    await user.keyboard("99{Enter}");
    await waitFor(() => expect(pageInput.value).toBe("12"), { timeout: 10_000 });
    expect(next).toBeDisabled();
    await user.click(prev);
    await waitFor(() => expect(pageInput.value).toBe("11"), { timeout: 10_000 });

    // Invalid input reverts on blur, while clicking Prev/Next does not commit draft digits.
    await user.click(pageInput);
    await user.keyboard("abc");
    scrollRegion.focus();
    await waitFor(() => expect(pageInput.value).toBe("11"));

    await user.click(pageInput);
    await user.keyboard("2");
    await user.click(prev);
    await waitFor(() => expect(pageInput.value).toBe("10"), { timeout: 10_000 });

    // Blurring after typing a valid number jumps (needed on touch numeric keypads without Enter).
    await user.click(pageInput);
    await user.keyboard("4");
    scrollRegion.focus();
    await waitFor(() => expect(pageInput.value).toBe("4"), { timeout: 10_000 });
  },
);

it("does not jump while IME composition is active", async () => {
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  fireEvent.focus(pageInput);
  fireEvent.compositionStart(pageInput);
  fireEvent.change(pageInput, { target: { value: "5" } });
  fireEvent.keyDown(pageInput, { key: "Enter", isComposing: true });
  fireEvent.submit(pageInput.closest("form")!);
  expect(pageInput.value).toBe("5");
  expect(screen.getByRole("button", { name: "前のページ" })).toBeDisabled();

  fireEvent.compositionEnd(pageInput);
  fireEvent.keyDown(pageInput, { key: "Enter", isComposing: false });
  fireEvent.submit(pageInput.closest("form")!);
  await waitFor(
    () => expect(screen.getByRole("button", { name: "前のページ" })).not.toBeDisabled(),
    { timeout: 10_000 },
  );
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
