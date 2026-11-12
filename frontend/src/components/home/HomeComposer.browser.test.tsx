import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { HomeComposer } from "./HomeComposer";
import { loadHomePostSettings, saveHomePostSettings } from "../../lib/homePreferences";

const fixture = vi.hoisted(() => ({
  channels: [
    { id: "other", channel_name: "other", is_private: false, is_dm: false, archive_at: null },
  ],
  joined: new Set(["other"]),
  syncing: true,
}));
vi.mock("../../hooks/useRealtime", () => ({
  useActiveChannels: () => ({
    data: fixture.channels,
    isLoading: false,
    isInitialSyncLoading: fixture.syncing,
  }),
  useMyChannelMemberships: () => ({
    joinedChannelIds: fixture.joined,
    isLoading: false,
    isInitialSyncLoading: fixture.syncing,
  }),
}));
vi.mock("../../lib/electric/collections-context", async (original) => ({
  ...(await original<typeof import("../../lib/electric/collections-context")>()),
  useCollections: () => ({ config: {} }),
}));
vi.mock("../layout/useMediaQuery", () => ({ useMediaQuery: () => true }));

beforeEach(() => {
  fixture.channels = [
    { id: "other", channel_name: "other", is_private: false, is_dm: false, archive_at: null },
  ];
  fixture.joined = new Set(["other"]);
  fixture.syncing = true;
  saveHomePostSettings("posting-target-test", {
    visibility: "public",
    apub: false,
    channel_id: "saved",
  });
});

it.each([true, false])(
  "waits for an uncached posting target before submitting (found: %s)",
  async (found) => {
    const post = vi.fn(async () => {});
    const view = () => (
      <LocaleProvider>
        <HomeComposer settingsKey="posting-target-test" federationEnabled={false} onPost={post} />
      </LocaleProvider>
    );
    const { rerender } = render(view());
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "投稿を書く" }));
    await user.type(
      await screen.findByRole("textbox", { name: "近況を投稿" }),
      "Saved target post",
    );
    expect(screen.getByRole("button", { name: "投稿" }).hasAttribute("disabled")).toBe(true);
    expect(loadHomePostSettings("posting-target-test").channel_id).toBe("saved");
    if (found) {
      fixture.channels = [
        ...fixture.channels,
        { ...fixture.channels[0], id: "saved", channel_name: "saved" },
      ];
      fixture.joined = new Set(["other", "saved"]);
    }
    fixture.syncing = false;
    rerender(view());
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "投稿" }).hasAttribute("disabled")).toBe(false),
    );
    expect(
      (screen.getByRole("combobox", { name: "同期先チャンネル" }) as HTMLSelectElement).value,
    ).toBe(found ? "saved" : "");
    if (!found) expect(loadHomePostSettings("posting-target-test").channel_id).toBeNull();
    await user.click(screen.getByRole("button", { name: "投稿" }));
    expect(post).toHaveBeenCalledWith(
      expect.objectContaining({ channel_id: found ? "saved" : null }),
    );
  },
);

it("preserves a per-image label selected during upload and submits only that image ID", async () => {
  await startMockServer();
  let completeUpload!: () => void;
  const pendingUpload = new Promise<void>((resolve) => {
    completeUpload = resolve;
  });
  mockServer.use(
    http.post("*/api/home/images", async ({ request }) => {
      const body = (await request.json()) as { name: string };
      if (body.name === "second.png") await pendingUpload;
      return HttpResponse.json({ id: body.name });
    }),
    http.delete("*/api/home/images/:id", () => new HttpResponse(null, { status: 204 })),
  );
  fixture.syncing = false;
  const post = vi.fn(async () => {});
  const { container } = render(
    <LocaleProvider>
      <HomeComposer settingsKey={null} federationEnabled={false} onPost={post} />
    </LocaleProvider>,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "投稿を書く" }));
  const input =
    container.querySelector<HTMLInputElement>('input[type="file"]') ??
    document.querySelector<HTMLInputElement>('input[type="file"]')!;
  await user.upload(
    input,
    ["first.png", "second.png"].map(
      (name) => new File([new Uint8Array([1, 2, 3, 4])], name, { type: "image/png" }),
    ),
  );
  const second = await screen.findByRole("checkbox", {
    name: "second.png: センシティブ",
  });
  await user.click(second);
  expect(screen.getByRole("checkbox", { name: "first.png: センシティブ" })).not.toBeChecked();
  completeUpload();
  await waitFor(() => expect(screen.getByRole("button", { name: "投稿" })).toBeEnabled());
  await user.click(screen.getByRole("button", { name: "投稿" }));
  await waitFor(() =>
    expect(post).toHaveBeenCalledWith(
      expect.objectContaining({
        image_ids: ["first.png", "second.png"],
        sensitive_image_ids: ["second.png"],
      }),
    ),
  );
});
