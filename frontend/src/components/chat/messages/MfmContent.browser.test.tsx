import { render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../../test/browser-setup";
import { MfmContent } from "./MfmContent";
import "../../../styles.css";

it.each([160, 640])(
  "keeps the next message in place while joined wide emoji bytes load at %ipx",
  async (width) => {
    await startMockServer();
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    let metadataRequests = 0;
    mockServer.use(
      http.get("*/api/emojis", () => {
        metadataRequests++;
        return HttpResponse.json([]);
      }),
      http.get("*/joined-wide-:width.png", async () => {
        await pending;
        return HttpResponse.arrayBuffer(
          Uint8Array.from(
            atob(
              "iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
            ),
            (c) => c.charCodeAt(0),
          ).buffer,
          { headers: { "content-type": "image/png" } },
        );
      }),
    );
    const view = render(
      <div style={{ width }}>
        <div className="mfm-body whitespace-pre-wrap text-sm leading-relaxed">
          <MfmContent
            content="Hello :wide: world"
            emojis={{ wide: { url: `/joined-wide-${width}.png`, width: 3, height: 1 } }}
          />
        </div>
        <div data-testid="following-message">Next message</div>
      </div>,
    );
    const image = screen.getByRole("img", { name: ":wide:" }) as HTMLImageElement;
    const marker = screen.getByTestId("following-message");
    const initial = marker.getBoundingClientRect().top;
    expect(image.naturalWidth).toBe(0);
    expect(image.getBoundingClientRect().width / image.getBoundingClientRect().height).toBeCloseTo(
      3,
    );
    release();
    await waitFor(() => expect(image.naturalWidth).toBe(3));
    expect(marker.getBoundingClientRect().top).toBe(initial);
    expect(metadataRequests).toBe(0);
    view.unmount();
  },
);

it.each([160, 320])("wraps long inline URLs within a %ipx message without padding", (width) => {
  const url = `https://example.com/${"verylong".repeat(30)}`;
  const view = render(
    <div
      style={{ width }}
      className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed"
    >
      <MfmContent content={url} />
    </div>,
  );
  const body = view.container.firstElementChild!;
  const link = screen.getByRole("link");
  expect(link).toHaveAttribute("href", url);
  expect(getComputedStyle(link).paddingTop).toBe("0px");
  expect(getComputedStyle(link).paddingBottom).toBe("0px");
  expect(getComputedStyle(link).cursor).toBe("pointer");
  expect(link.getClientRects().length).toBeGreaterThan(1);
  expect(body.scrollWidth).toBeLessThanOrEqual(body.clientWidth);
});

it.each([160, 640])("keeps heading size, wrapping and spacing within a %ipx message", (width) => {
  const view = render(
    <div
      style={{ width }}
      className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed"
    >
      <MfmContent content={`# ${"LongHeading".repeat(12)}\n## Smaller\nBody`} />
    </div>,
  );
  const [large, small] = screen.getAllByRole("heading");
  expect(parseFloat(getComputedStyle(large).fontSize)).toBeGreaterThan(
    parseFloat(getComputedStyle(small).fontSize),
  );
  expect(getComputedStyle(large).marginTop).toBe("0px");
  const body = view.container.firstElementChild!;
  expect(body.scrollWidth).toBeLessThanOrEqual(body.clientWidth);
});
