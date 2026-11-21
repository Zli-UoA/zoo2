import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { createRef } from "react";
import { HomeComposer, type HomeComposerHandle } from "./HomeComposer";
import { loadHomePostSettings, saveHomePostSettings } from "../../lib/homePreferences";

type ChannelFixture = {
  id: string;
  channel_name: string;
  is_private: boolean;
  is_dm: boolean;
  archive_at: null;
  created_user_id?: string;
  posting_policy?: "members" | "managers";
};
const fixture = vi.hoisted(() => ({
  channels: [
    { id: "other", channel_name: "other", is_private: false, is_dm: false, archive_at: null },
  ] as ChannelFixture[],
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
// 現在のユーザーの取得は、このテストの対象ではない（毎回新しい config を返すモックと組み合わせると、
// config ごとのストアが描画のたびに作り直される）。
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isAdmin: false, isLoading: false, error: null }),
}));

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

/** 管理者のみ投稿できるチャンネル。作成者は別の人で、現在の利用者（me）は管理者ではない。 */
const managersChannel: ChannelFixture = {
  id: "managed",
  channel_name: "managed",
  is_private: false,
  is_dm: false,
  archive_at: null,
  created_user_id: "owner",
  posting_policy: "managers",
};
async function replyToManagedChannel(joined: boolean) {
  fixture.syncing = false;
  fixture.channels = [managersChannel];
  fixture.joined = new Set(joined ? ["managed"] : []);
  const handle = createRef<HomeComposerHandle>();
  render(
    <LocaleProvider>
      <HomeComposer
        ref={handle}
        settingsKey={null}
        federationEnabled={false}
        onPost={vi.fn(async () => {})}
      />
    </LocaleProvider>,
  );
  await act(async () =>
    handle.current!.reply({
      id: "post",
      display_name: "Owner",
      content: "hello",
      channel_id: "managed",
    }),
  );
  const user = userEvent.setup();
  await user.type(await screen.findByRole("textbox", { name: "近況を投稿" }), "reply");
}

it("stops a reply to a managers-only channel the user has joined, and says why", async () => {
  await replyToManagedChannel(true);
  expect(await screen.findByText("このチャンネルは管理者のみ投稿できます。")).toBeVisible();
  expect(screen.getByRole("button", { name: "投稿" }).hasAttribute("disabled")).toBe(true);
});

it("does not stop a reply from someone who has not joined the channel", async () => {
  // The server takes it as a reply of Home only; the channel's posting policy does not apply.
  await replyToManagedChannel(false);
  expect(screen.queryByText("このチャンネルは管理者のみ投稿できます。")).not.toBeInTheDocument();
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "投稿" }).hasAttribute("disabled")).toBe(false),
  );
});
