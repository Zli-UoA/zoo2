import "../../../styles.css";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessageAttachments } from "./MessageAttachments";
import type { AttachmentUrlState } from "../../../hooks/useAttachments";

const link = vi.hoisted(() => ({ state: {} as AttachmentUrlState, load: vi.fn() }));
vi.mock("../../../hooks/useAttachments", () => ({
  useAttachmentUrl: () => {
    link.load();
    return link.state;
  },
}));

it.each([160, 640])(
  "keeps attachment geometry through signed URL loading, wide image decode and failure at %ipx",
  async (width) => {
    link.state = {
      url: null,
      status: "loading",
      expiresAt: null,
      reload: vi.fn(),
      getFreshUrl: vi.fn(),
    };
    const attachment = {
      id: "file",
      message_id: "message",
      uploader_id: "alice",
      channel_id: "channel",
      file_name: "wide.png",
      width: 3,
      height: 1,
      blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
      file_size: 100,
      content_type: "image/png",
      storage_key: "file",
      created_at: "2026-11-02 00:00:00.000",
    };
    const element = () => (
      <LocaleProvider>
        <div style={{ width }}>
          <MessageAttachments attachments={[attachment]} />
          <div data-testid="following">Next message</div>
        </div>
      </LocaleProvider>
    );
    const view = render(element());
    const top = screen.getByTestId("following").getBoundingClientRect().top;
    link.state = {
      ...link.state,
      status: "ready",
      url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
    };
    view.rerender(element());
    const image = screen.getByRole("img", { name: "wide.png" }) as HTMLImageElement;
    await waitFor(() => expect(image.naturalWidth).toBe(3));
    expect(screen.getByTestId("following").getBoundingClientRect().top).toBe(top);
    link.state = { ...link.state, status: "error" };
    view.rerender(element());
    expect(screen.getByTestId("following").getBoundingClientRect().top).toBe(top);
  },
);

it("shows BlurHash without requesting the original image until a sensitive image is tapped", async () => {
  link.load.mockClear();
  link.state = {
    url: "/sensitive.png",
    status: "ready",
    expiresAt: null,
    reload: vi.fn(),
    getFreshUrl: vi.fn(),
  };
  const attachment = {
    id: "sensitive",
    width: 640,
    height: 480,
    blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
    file_name: "private.png",
    file_size: 100,
    content_type: "image/png",
    is_sensitive: true,
  };
  render(
    <LocaleProvider>
      <MessageAttachments attachments={[attachment]} />
    </LocaleProvider>,
  );
  expect(link.load).not.toHaveBeenCalled();
  expect(screen.queryByRole("img")).toBeNull();
  const preview = screen.getByRole("button", { name: "画像を表示" }).querySelector("canvas")!;
  await waitFor(() => expect(preview.getContext("2d")!.getImageData(0, 0, 1, 1).data[3]).toBe(255));
  fireEvent.click(screen.getByRole("button", { name: "画像を表示" }));
  expect(await screen.findByRole("img", { name: "private.png" })).toBeTruthy();
  expect(link.load).toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "画像を隠す" }));
  expect(screen.queryByRole("img")).toBeNull();
});
