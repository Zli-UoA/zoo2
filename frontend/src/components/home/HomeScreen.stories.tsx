import { blobcat, misskeyPost } from "../../../.storybook/activitypub-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse, ws } from "msw";
import { expect, fireEvent, screen, userEvent, waitFor, within } from "storybook/test";
import { HomeScreen } from "./HomeScreen";
import { homePosts as posts, homeHandlers } from "../../../.storybook/home-fixtures";

const meta = {
  title: "Pages/Home",
  component: HomeScreen,
  parameters: {
    a11y: { test: "error" },
    msw: { handlers: homeHandlers },
  },
  decorators: [
    (Story) => (
      <div className="flex h-[720px] flex-col bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof HomeScreen>;
export default meta;
type Story = StoryObj<typeof meta>;
async function openComposer(canvasElement: HTMLElement) {
  await userEvent.click(within(canvasElement).getByRole("button", { name: "投稿を書く" }));
  const dialog = await screen.findByRole("dialog", { name: "近況を投稿" });
  const form = within(dialog);
  await waitFor(() => expect(getComputedStyle(dialog).opacity).toBe("1"));
  await waitFor(() => expect(form.getByRole("textbox", { name: "近況を投稿" })).toHaveFocus());
  return form;
}
async function expectComposerClosed() {
  await waitFor(() =>
    expect(screen.queryByRole("dialog", { name: "近況を投稿" })).not.toBeInTheDocument(),
  );
}

export const Default: Story = {
  play: async ({ canvasElement }) => {
    await expect(
      await within(canvasElement).findByText(/Homeから最初の公開投稿です/),
    ).toBeVisible();
    await expect(
      within(canvasElement).queryByRole("textbox", { name: "近況を投稿" }),
    ).not.toBeInTheDocument();
    await expect(within(canvasElement).getByRole("button", { name: "投稿を書く" })).toBeVisible();
    await expect(
      within(canvasElement).queryByText("ActivityPub・チャンネル連携"),
    ).not.toBeInTheDocument();
    await expect(
      within(canvasElement).queryByText(/zoo\.example\/api\/activitypub\/users/),
    ).not.toBeInTheDocument();
  },
};
export const Mobile: Story = { ...Default, globals: { viewport: { value: "mobile" } } };
export const Empty: Story = {
  parameters: { msw: { handlers: [http.get("*/api/home", () => HttpResponse.json([]))] } },
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", async () => {
          await delay("infinite");
          return HttpResponse.json([]);
        }),
      ],
    },
  },
};
export const LoadError: Story = {
  parameters: {
    msw: { handlers: [http.get("*/api/home", () => new HttpResponse(null, { status: 500 }))] },
  },
};
let submittedContent: string | null = null;
export const Submit: Story = {
  beforeEach: () => {
    submittedContent = null;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json(
            submittedContent
              ? [{ ...posts[0], id: "new-post", content: submittedContent }, ...posts]
              : posts,
          ),
        ),
        http.post("*/api/home", async ({ request }) => {
          const body = (await request.json()) as {
            content: string;
            apub: boolean;
            visibility: string;
          };
          submittedContent = body.content;
          return body.content === "公開する近況" && body.apub && body.visibility === "unlisted"
            ? HttpResponse.json({ id: "new-post" }, { status: 201 })
            : new HttpResponse(null, { status: 400 });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const form = await openComposer(canvasElement);
    const field = form.getByRole("textbox", { name: "近況を投稿" });
    await userEvent.type(field, "公開する近況");
    await userEvent.click(form.getByRole("button", { name: /^投稿$/ }));
    await expectComposerClosed();
    await expect(await canvas.findByText("公開する近況")).toBeVisible();
  },
};
export const SendError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(posts)),
        http.post("*/api/home", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    const field = form.getByRole("textbox", { name: "近況を投稿" });
    await userEvent.type(field, "残しておく下書き");
    await userEvent.click(form.getByRole("button", { name: /^投稿$/ }));
    await expect(await form.findByRole("alert")).toBeVisible();
    await expect(field).toHaveValue("残しておく下書き");
  },
};

export const OlderPosts: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", ({ request }) => {
          if (new URL(request.url).searchParams.has("before")) {
            return HttpResponse.json([
              { ...posts[1], id: "older", content: "過去の投稿を読み込みました" },
            ]);
          }
          return HttpResponse.json(
            Array.from({ length: 50 }, (_, i) => ({ ...posts[0], id: `post-${i}` })),
          );
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "過去の投稿を読む" }));
    await expect(await canvas.findByText("過去の投稿を読み込みました")).toBeVisible();
    await expect(
      canvas.queryByRole("button", { name: "過去の投稿を読む" }),
    ).not.toBeInTheDocument();
  },
};
let deleted = false;
export const DeletePost: Story = {
  beforeEach: () => {
    deleted = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(deleted ? posts.slice(1) : posts)),
        http.delete("*/api/home/home-post", () => {
          deleted = true;
          return new HttpResponse(null, { status: 204 });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      within(
        (await canvas.findByText(/Homeから最初の公開投稿です。/)).closest("article")!,
      ).getByRole("button", { name: "投稿の操作" }),
    );
    await userEvent.click(await screen.findByRole("menuitem", { name: "投稿を削除" }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "投稿を削除" }));
    await waitFor(() =>
      expect(canvas.queryByText(/Homeから最初の公開投稿です/)).not.toBeInTheDocument(),
    );
  },
};

export const Federated: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json([
            {
              ...posts[0],
              apub: true,
              visibility: "public",
              channel_id: "channel-general",
              channel_name: "general",
            },
            {
              ...posts[1],
              id: "https://social.example/notes/1",
              user_id: "https://social.example/users/bob",
              apub: true,
              visibility: "followers",
              remote: true,
              url: "https://social.example/notes/1",
              content: "外部のフォロー先から届いた投稿です。",
            },
          ]),
        ),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findByText("外部のフォロー先から届いた投稿です。");
    await expect(canvas.queryByRole("img", { name: "ActivityPub" })).not.toBeInTheDocument();
    const channel = canvas.getByRole("link", { name: "#general" });
    await expect(channel.previousElementSibling?.tagName).toBe("TIME");
    await expect(canvas.getAllByRole("link", { name: "#general" })).toHaveLength(1);
  },
};
let crossPosted = false;
export const CrossPost: Story = {
  beforeEach: () => {
    crossPosted = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json(
            crossPosted
              ? [
                  {
                    ...posts[0],
                    content: "同じ投稿を連合とチャンネルに",
                    apub: true,
                    visibility: "public",
                    channel_id: "channel-general",
                    channel_name: "general",
                  },
                ]
              : posts,
          ),
        ),
        http.post("*/api/home", async ({ request }) => {
          const body = (await request.json()) as {
            apub: boolean;
            visibility: string;
            channel_id: string;
          };
          crossPosted =
            body.apub && body.visibility === "public" && body.channel_id === "channel-general";
          return crossPosted
            ? HttpResponse.json({ id: "cross-post" }, { status: 201 })
            : new HttpResponse(null, { status: 400 });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const form = await openComposer(canvasElement);
    await userEvent.click(form.getByRole("button", { name: /^公開範囲:/ }));
    const menu = await screen.findByRole("menu");
    await waitFor(() =>
      expect(within(menu).getByRole("menuitemradio", { name: "パブリック" })).toBeEnabled(),
    );
    await userEvent.click(within(menu).getByRole("menuitemradio", { name: "パブリック" }));
    await userEvent.selectOptions(
      form.getByRole("combobox", { name: "同期先チャンネル" }),
      "channel-general",
    );
    await userEvent.type(
      form.getByRole("textbox", { name: "近況を投稿" }),
      "同じ投稿を連合とチャンネルに",
    );
    await userEvent.click(form.getByRole("button", { name: /^投稿$/ }));
    await expectComposerClosed();
    await expect(await canvas.findByText("同じ投稿を連合とチャンネルに")).toBeVisible();
    await expect(canvas.queryByRole("img", { name: "ActivityPub" })).not.toBeInTheDocument();
    await expect(canvas.getByRole("link", { name: "#general" })).toBeVisible();
    const channel = canvas.getByRole("link", { name: "#general" });
    await expect(channel.previousElementSibling?.tagName).toBe("TIME");
  },
};
export const FollowersOnly: Story = {
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    const channel = form.getByRole("combobox", { name: "同期先チャンネル" });
    await userEvent.selectOptions(channel, "channel-general");
    await userEvent.click(form.getByRole("button", { name: /^公開範囲:/ }));
    const menu = await screen.findByRole("menu");
    await waitFor(() =>
      expect(within(menu).getByRole("menuitemradio", { name: "フォロワー限定" })).toBeEnabled(),
    );
    await userEvent.click(within(menu).getByRole("menuitemradio", { name: "フォロワー限定" }));
    await expect(channel).toBeDisabled();
    await expect(channel).toHaveValue("");
    await waitFor(() =>
      expect(document.querySelector('[data-slot="dropdown-menu-content"]')).not.toBeInTheDocument(),
    );
  },
};
let followed = false;
export const FollowRemote: Story = {
  beforeEach: () => {
    followed = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(posts)),
        http.get("*/api/activitypub/follows", () =>
          HttpResponse.json(
            followed
              ? [
                  {
                    actor_id: "https://social.example/users/bob",
                    display_name: "Bob @ social.example",
                    direction: "outgoing",
                    accepted: false,
                  },
                ]
              : [],
          ),
        ),
        http.get("*/api/activitypub/search", () =>
          HttpResponse.json([
            {
              id: "https://social.example/users/bob",
              user_name: "bob",
              display_name: "Bob @ social.example",
              remote: true,
              url: "https://social.example/users/bob",
            },
          ]),
        ),
        http.post("*/api/activitypub/follows", () => {
          followed = true;
          return HttpResponse.json({ actor_id: "https://social.example/users/bob" });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "ユーザーを検索" }));
    const dialog = within(await screen.findByRole("dialog", { name: "ユーザーを検索" }));
    const account = dialog.getByRole("textbox", { name: "ユーザーを検索" });
    await userEvent.type(account, "@bob@social.example");
    await userEvent.click(dialog.getByRole("button", { name: "検索" }));
    const details = within(await dialog.findByRole("article", { name: "ユーザー詳細" }));
    await expect(details.getByText("Bob @ social.example")).toBeVisible();
    await userEvent.click(details.getByRole("button", { name: /^フォロー$/ }));
    await expect(await details.findByText("承認待ち")).toBeVisible();
  },
};

let editedContent: string | null = null;
export const EditPost: Story = {
  beforeEach: () => {
    editedContent = null;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json([
            { ...posts[0], content: editedContent ?? posts[0].content },
            posts[1],
          ]),
        ),
        http.patch("*/api/home/home-post", async ({ request }) => {
          const body = (await request.json()) as { content: string };
          editedContent = body.content;
          return new HttpResponse(null, { status: 204 });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      within(
        (await canvas.findByText(/Homeから最初の公開投稿です。/)).closest("article")!,
      ).getByRole("button", { name: "投稿の操作" }),
    );
    await userEvent.click(await screen.findByRole("menuitem", { name: "投稿を編集" }));
    const field = canvas.getByRole("textbox", { name: "投稿を編集" });
    await userEvent.clear(field);
    await userEvent.type(field, "編集した近況です");
    await userEvent.click(canvas.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(canvas.queryByRole("textbox", { name: "投稿を編集" })).not.toBeInTheDocument(),
    );
    await expect(await canvas.findByText("編集した近況です")).toBeVisible();
  },
};
export const FederationError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(posts)),
        http.get("*/api/activitypub/status", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "ユーザーを検索" }));
    const dialog = within(await screen.findByRole("dialog", { name: "ユーザーを検索" }));
    await expect(await dialog.findByRole("alert")).toBeVisible();
  },
};

export const VisibilityMenu: Story = {
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    const toolbar = form.getByRole("group", { name: "投稿設定" });
    const buttons = within(toolbar).getAllByRole("button");
    await expect(buttons).toHaveLength(3);
    await expect(buttons.map((button) => button.getAttribute("aria-label"))).toEqual([
      "公開範囲: ホーム",
      "連合あり",
      "投稿",
    ]);
    await expect(buttons.every((button) => !button.textContent?.trim())).toBe(true);
    await userEvent.click(form.getByRole("button", { name: /^公開範囲:/ }));
    const menu = await screen.findByRole("menu");
    const items = within(menu).getAllByRole("menuitemradio");
    await expect(items.map((item) => item.textContent)).toEqual([
      "パブリック",
      "ホーム",
      "フォロワー限定",
    ]);
    await expect(within(menu).getByRole("menuitemradio", { name: "ホーム" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(items.every((item) => item.querySelector("svg"))).toBe(true);
  },
};

export const Composer: Story = {
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    await expect(form.getByRole("group", { name: "投稿設定" })).toBeVisible();
    const box = screen.getByRole("dialog", { name: "近況を投稿" }).getBoundingClientRect();
    await expect(box.left).toBeGreaterThan(0);
    await expect(box.width).toBeLessThan(window.innerWidth);
  },
};
export const MobileComposer: Story = {
  globals: { viewport: { value: "mobile" } },
  play: async ({ canvasElement }) => {
    let focusedDuringClick = false;
    const captureFocus = () => {
      focusedDuringClick = document.activeElement?.getAttribute("aria-label") === "近況を投稿";
    };
    document.addEventListener("click", captureFocus);
    try {
      await openComposer(canvasElement);
    } finally {
      document.removeEventListener("click", captureFocus);
    }
    await expect(focusedDuringClick).toBe(true);
    await waitFor(async () => {
      const box = screen.getByRole("dialog", { name: "近況を投稿" }).getBoundingClientRect();
      await expect(Math.abs(box.left)).toBeLessThan(1);
      await expect(Math.abs(box.top)).toBeLessThan(1);
      await expect(Math.abs(box.width - window.innerWidth)).toBeLessThan(1);
      await expect(Math.abs(box.height - window.innerHeight)).toBeLessThan(1);
    });
    const viewport = window.visualViewport!;
    const original = Object.getOwnPropertyDescriptor(viewport, "height");
    const keyboardHeight = window.innerHeight - 300;
    try {
      Object.defineProperty(viewport, "height", { configurable: true, value: keyboardHeight });
      viewport.dispatchEvent(new Event("resize"));
      await waitFor(async () => {
        const box = screen.getByRole("dialog", { name: "近況を投稿" }).getBoundingClientRect();
        await expect(Math.abs(box.height - keyboardHeight)).toBeLessThan(1);
        await expect(Math.abs(box.top)).toBeLessThan(1);
      });
    } finally {
      if (original) Object.defineProperty(viewport, "height", original);
      else Reflect.deleteProperty(viewport, "height");
      viewport.dispatchEvent(new Event("resize"));
    }
  },
};
export const DraftPreserved: Story = {
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    await userEvent.type(form.getByRole("textbox", { name: "近況を投稿" }), "途中まで書いた下書き");
    await userEvent.click(form.getByRole("button", { name: "閉じる" }));
    await expectComposerClosed();
    const reopened = await openComposer(canvasElement);
    await expect(reopened.getByRole("textbox", { name: "近況を投稿" })).toHaveValue(
      "途中まで書いた下書き",
    );
  },
};
let localPost = false;
export const FederationToggle: Story = {
  beforeEach: () => {
    localPost = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json(localPost ? [{ ...posts[0], content: "連合しないホーム投稿" }] : posts),
        ),
        http.post("*/api/home", async ({ request }) => {
          const body = (await request.json()) as {
            apub: boolean;
            visibility: string;
            content: string;
          };
          localPost =
            !body.apub && body.visibility === "unlisted" && body.content === "連合しないホーム投稿";
          return localPost
            ? HttpResponse.json({ id: "local" }, { status: 201 })
            : new HttpResponse(null, { status: 400 });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    const toggle = await form.findByRole("button", { name: "連合あり" });
    await userEvent.click(toggle);
    await expect(await screen.findByRole("menuitemradio", { name: "連合あり" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await userEvent.click(screen.getByRole("menuitemradio", { name: "連合なし" }));
    await expect(form.getByRole("button", { name: "連合なし" })).toBeVisible();
    await expect(form.getByRole("button", { name: "公開範囲: ホーム" })).toBeVisible();
    await userEvent.type(form.getByRole("textbox", { name: "近況を投稿" }), "連合しないホーム投稿");
    await userEvent.click(form.getByRole("button", { name: /^投稿$/ }));
    await expectComposerClosed();
    await expect(await within(canvasElement).findByText("連合しないホーム投稿")).toBeVisible();
  },
};
export const FederationUnavailable: Story = {
  ...FederationToggle,
  parameters: {
    msw: {
      handlers: [
        ...FederationToggle.parameters!.msw.handlers,
        http.get("*/api/activitypub/status", () => HttpResponse.json({ enabled: false })),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    await userEvent.click(form.getByRole("button", { name: "連合なし" }));
    await expect(await screen.findByRole("menuitemradio", { name: "連合あり" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await userEvent.click(screen.getByRole("menuitemradio", { name: "連合なし" }));
    await userEvent.type(form.getByRole("textbox", { name: "近況を投稿" }), "連合しないホーム投稿");
    await userEvent.click(form.getByRole("button", { name: /^投稿$/ }));
    await expectComposerClosed();
    await expect(await within(canvasElement).findByText("連合しないホーム投稿")).toBeVisible();
  },
};

export const Sending: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(posts)),
        http.post("*/api/home", async () => {
          await delay("infinite");
          return HttpResponse.json({ id: "pending" });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const form = await openComposer(canvasElement);
    await userEvent.type(form.getByRole("textbox", { name: "近況を投稿" }), "送信中の投稿");
    await userEvent.click(form.getByRole("button", { name: /^投稿$/ }));
    await expect(await form.findByRole("button", { name: "投稿中…" })).toBeDisabled();
    await expect(form.getByRole("button", { name: "閉じる" })).toBeDisabled();
    await userEvent.keyboard("{Escape}");
    await expect(screen.getByRole("dialog", { name: "近況を投稿" })).toBeVisible();
  },
};

export const UserSearch: Story = {
  parameters: {
    msw: {
      handlers: [
        ...homeHandlers,
        http.get("*/api/activitypub/search", () =>
          HttpResponse.json([
            {
              id: "user-alice",
              user_name: "alice",
              display_name: "Alice",
              remote: false,
              created_at: "2026-01-01T00:00:00Z",
            },
          ]),
        ),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getByRole("button", { name: "ユーザーを検索" }));
    const dialog = within(await screen.findByRole("dialog", { name: "ユーザーを検索" }));
    await userEvent.type(dialog.getByRole("textbox"), "alice");
    await userEvent.click(dialog.getByRole("button", { name: "検索" }));
    const details = within(await dialog.findByRole("article", { name: "ユーザー詳細" }));
    await expect(details.getByText("Alice")).toBeVisible();
    await expect(details.getByText(/登録日:/)).toBeVisible();
  },
};

const live = ws.link("*/api/home/live");
let livePosts = posts;
let liveAuthenticated = false;
export const Realtime: Story = {
  beforeEach: () => {
    livePosts = posts;
    liveAuthenticated = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(livePosts)),
        live.addEventListener("connection", ({ client }) => {
          client.addEventListener("message", () => {
            liveAuthenticated = true;
            client.send("refresh");
          });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText(/Homeから最初の公開投稿です。/)).toBeVisible();
    await waitFor(() => expect(liveAuthenticated).toBe(true));
    livePosts = [{ ...posts[0], id: "live-post", content: "別端末から届いた新しい投稿" }, ...posts];
    live.broadcast("refresh");
    await expect(await canvas.findByText("別端末から届いた新しい投稿")).toBeVisible();
    livePosts = [
      { ...posts[0], id: "live-post", content: "編集がリアルタイムで反映されます" },
      posts[1],
    ];
    live.broadcast("refresh");
    await expect(await canvas.findByText("編集がリアルタイムで反映されます")).toBeVisible();
    await waitFor(() =>
      expect(canvas.queryByText(/Homeから最初の公開投稿です。/)).not.toBeInTheDocument(),
    );
    livePosts = [{ ...posts[0], id: "reconnected", content: "再接続後の最新投稿" }];
    for (const client of live.clients) client.close(1000);
    await expect(
      await canvas.findByText("再接続後の最新投稿", {}, { timeout: 5000 }),
    ).toBeVisible();
  },
};

let pullRefreshed = false;
export const PullToRefresh: Story = {
  globals: { viewport: { value: "mobile" } },
  beforeEach: () => {
    pullRefreshed = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json(
            pullRefreshed
              ? [{ ...posts[0], content: "引っ張って最新の投稿を取得しました" }]
              : posts,
          ),
        ),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findByText(/Homeから最初の公開投稿です。/);
    const root = canvasElement.querySelector(".overflow-y-auto")!;
    await fireEvent.touchStart(root, {
      touches: [new Touch({ identifier: 1, target: root, clientY: 20 })],
    });
    await fireEvent.touchMove(root, {
      touches: [new Touch({ identifier: 1, target: root, clientY: 180 })],
    });
    await expect(await canvas.findByText("離して更新")).toBeVisible();
    pullRefreshed = true;
    await fireEvent.touchEnd(root, { touches: [] });
    await expect(await canvas.findByText("引っ張って最新の投稿を取得しました")).toBeVisible();
    await waitFor(() => expect(canvas.queryByRole("status")).not.toBeInTheDocument());
  },
};

export const UserSearchEmpty: Story = {
  parameters: {
    msw: {
      handlers: [
        ...homeHandlers,
        http.get("*/api/activitypub/search", () => HttpResponse.json([])),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getByRole("button", { name: "ユーザーを検索" }));
    const dialog = within(await screen.findByRole("dialog", { name: "ユーザーを検索" }));
    await userEvent.type(dialog.getByRole("textbox"), "nobody");
    await userEvent.click(dialog.getByRole("button", { name: "検索" }));
    await expect(await dialog.findByText("ユーザーが見つかりません")).toBeVisible();
  },
};

export const UserSearchError: Story = {
  parameters: {
    msw: {
      handlers: [...homeHandlers, http.get("*/api/activitypub/search", () => HttpResponse.error())],
    },
  },
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getByRole("button", { name: "ユーザーを検索" }));
    const dialog = within(await screen.findByRole("dialog", { name: "ユーザーを検索" }));
    await userEvent.type(dialog.getByRole("textbox"), "@bob@social.example");
    await userEvent.click(dialog.getByRole("button", { name: "検索" }));
    await expect(await dialog.findByRole("alert")).toBeVisible();
  },
};

export const TimelineSwitch: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", ({ request }) => {
          const timeline = new URL(request.url).searchParams.get("timeline");
          return HttpResponse.json([
            {
              ...posts[0],
              content:
                timeline === "social"
                  ? "ソーシャルの投稿"
                  : timeline === "local"
                    ? "ローカルの投稿"
                    : "ホームの投稿",
            },
          ]);
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    for (const name of ["ホーム", "ローカル", "ソーシャル", "ホーム"]) {
      await userEvent.click(canvas.getByRole("button", { name }));
      await expect(await canvas.findByText(`${name}の投稿`)).toBeVisible();
      await expect(canvas.getByRole("button", { name })).toHaveAttribute("aria-pressed", "true");
    }
  },
};

let liked = false;
export const Like: Story = {
  beforeEach: () => {
    liked = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json([
            {
              ...posts[0],
              likes: liked ? 1 : 0,
              liked,
              reactions: liked ? [{ reaction: "❤️", emoji: null, count: 1, selected: true }] : [],
            },
          ]),
        ),
        http.post("*/api/activitypub/reactions", async ({ request }) => {
          const body = (await request.json()) as { post_id: string };
          await expect(body.post_id).toBe(posts[0].id);
          liked = true;
          return HttpResponse.json({ ok: true });
        }),
        http.delete("*/api/activitypub/reactions", () => {
          liked = false;
          return HttpResponse.json({ ok: true });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "リアクション 0" }));
    const picker = within(await screen.findByRole("dialog", { name: "リアクション" }));
    await userEvent.click(
      within(picker.getByRole("group", { name: "よく使うリアクション" })).getByRole("button", {
        name: "❤️",
      }),
    );
    await expect(await canvas.findByRole("button", { name: "❤️ 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await userEvent.click(canvas.getByRole("button", { name: "❤️ 1" }));
    await waitFor(() =>
      expect(canvas.queryByRole("button", { name: "❤️ 1" })).not.toBeInTheDocument(),
    );
    await expect(canvas.getByRole("button", { name: "リアクション 0" })).toBeVisible();
  },
};
export const LikeError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json([posts[0]])),
        http.post("*/api/activitypub/reactions", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "リアクション 0" }));
    const picker = within(await screen.findByRole("dialog", { name: "リアクション" }));
    await userEvent.click(
      within(picker.getByRole("group", { name: "よく使うリアクション" })).getByRole("button", {
        name: "❤️",
      }),
    );
    await expect(await canvas.findByRole("alert")).toBeVisible();
  },
};

export const ActionBar: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const group = await canvas.findByRole("group", { name: "投稿のアクション" });
    await expect(
      within(group)
        .getAllByRole("button")
        .map((button) => button.getAttribute("aria-label")),
    ).toEqual(["返信 0", "リポスト 0", "リアクション 0", "投稿の操作"]);
    await expect(group.querySelector("time")).toBeNull();
    await userEvent.click(within(group).getByRole("button", { name: "投稿の操作" }));
    await waitFor(() => expect(screen.getByRole("menuitem", { name: "投稿を編集" })).toBeVisible());
  },
};
let sentReply = false;
export const Reply: Story = {
  beforeEach: () => {
    sentReply = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json(
            sentReply
              ? [
                  { ...posts[0], id: "reply", parent_id: posts[0].id, content: "返信のメッセージ" },
                  { ...posts[0], apub: true, visibility: "public", replies: 1 },
                ]
              : [{ ...posts[0], apub: true, visibility: "public" }],
          ),
        ),
        http.post("*/api/home", async ({ request }) => {
          const body = (await request.json()) as { in_reply_to: string; content: string };
          await expect(body.in_reply_to).toBe(posts[0].id);
          await expect(body.content).toBe("返信のメッセージ");
          sentReply = true;
          return HttpResponse.json({ id: "reply" }, { status: 201 });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    let focused = false;
    const observe = () => {
      focused = document.activeElement?.getAttribute("aria-label") === "近況を投稿";
    };
    document.addEventListener("click", observe);
    try {
      await userEvent.click(await canvas.findByRole("button", { name: "返信 0" }));
    } finally {
      document.removeEventListener("click", observe);
    }
    if (innerWidth < 768) await expect(focused).toBe(true);
    const form = within(await screen.findByRole("dialog", { name: "近況を投稿" }));
    await waitFor(() => expect(form.getByText("Aliceさんに返信")).toBeVisible());
    await userEvent.type(form.getByRole("textbox", { name: "近況を投稿" }), "返信のメッセージ");
    await userEvent.click(form.getByRole("button", { name: "投稿" }));
    await expectComposerClosed();
    await expect(await canvas.findByText("返信のメッセージ")).toBeVisible();
  },
};
export const MobileReply: Story = { ...Reply, globals: { viewport: { value: "mobile" } } };
export const PrivateReply: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () =>
          HttpResponse.json([{ ...posts[0], apub: true, visibility: "followers" }]),
        ),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("button", { name: "リポスト 0" })).toBeDisabled();
    await userEvent.click(canvas.getByRole("button", { name: "返信 0" }));
    const form = within(await screen.findByRole("dialog", { name: "近況を投稿" }));
    await userEvent.click(form.getByRole("button", { name: "公開範囲: フォロワー限定" }));
    await expect(await screen.findByRole("menuitemradio", { name: "パブリック" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await expect(screen.getByRole("menuitemradio", { name: "ホーム" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  },
};
let reposted = false;
export const Repost: Story = {
  beforeEach: () => {
    reposted = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => {
          const post = { ...posts[0], reposts: reposted ? 1 : 0, reposted };
          return HttpResponse.json(
            reposted ? [{ ...post, entry_id: "repost:mine", reposted_by: "Alice" }, post] : [post],
          );
        }),
        http.post("*/api/activitypub/reposts", () => {
          reposted = true;
          return HttpResponse.json({ ok: true });
        }),
        http.delete("*/api/activitypub/reposts", () => {
          reposted = false;
          return HttpResponse.json({ ok: true });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "リポスト 0" }));
    await expect(await canvas.findByText("Aliceさんがリポスト")).toBeVisible();
    const buttons = await canvas.findAllByRole("button", { name: "リポストを取り消す 1" });
    await userEvent.click(buttons[0]);
    await waitFor(() => expect(canvas.queryByText("Aliceさんがリポスト")).not.toBeInTheDocument());
  },
};

let customSelected = false;
const misskeyHandlers = [
  http.get("*/api/home", () =>
    HttpResponse.json([
      {
        ...misskeyPost,
        likes: customSelected ? 3 : 2,
        reactions: [
          {
            reaction: blobcat.name,
            emoji: blobcat,
            count: customSelected ? 3 : 2,
            selected: customSelected,
          },
        ],
      },
    ]),
  ),
  http.get("*/api/activitypub/reaction-options", () =>
    HttpResponse.json({ software: "misskey", emojis: [blobcat] }),
  ),
  http.post("*/api/activitypub/reactions", async ({ request }) => {
    const body = (await request.json()) as { reaction: string; emoji_id: string };
    await expect(body.reaction).toBe(blobcat.name);
    await expect(body.emoji_id).toBe(blobcat.image_id);
    customSelected = true;
    return HttpResponse.json({ ok: true });
  }),
  http.delete("*/api/activitypub/reactions", () => {
    customSelected = false;
    return HttpResponse.json({ ok: true });
  }),
];
export const MisskeyCustomEmoji: Story = {
  beforeEach: () => {
    customSelected = false;
  },
  parameters: { msw: { handlers: misskeyHandlers } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const host = await canvas.findByText("@misskey.example");
    await expect(host).toHaveClass("text-muted-foreground");
    await expect(host.parentElement).toHaveTextContent("@alice@misskey.example");
    const images = await canvas.findAllByRole("img", { name: ":blobcat:" });
    await expect(images).toHaveLength(3);
    await waitFor(() =>
      expect(images.every((image) => (image as HTMLImageElement).naturalWidth > 0)).toBe(true),
    );
    await expect(
      images.every((image) =>
        image.getAttribute("src")?.includes("/api/activitypub/emoji-images/"),
      ),
    ).toBe(true);
  },
};
export const MisskeyReact: Story = {
  ...MisskeyCustomEmoji,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "リアクション 2" }));
    const picker = within(await screen.findByRole("dialog", { name: "リアクション" }));
    await userEvent.click(await picker.findByRole("button", { name: ":blobcat:" }));
    const reaction = await canvas.findByRole("button", { name: ":blobcat: 3" });
    await expect(reaction).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(reaction);
    await expect(await canvas.findByRole("button", { name: ":blobcat: 2" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  },
};
export const MobileMisskey: Story = {
  ...MisskeyCustomEmoji,
  globals: { viewport: { value: "mobile" } },
};
export const EmojiFallback: Story = {
  ...MisskeyCustomEmoji,
  parameters: {
    msw: {
      handlers: [
        http.get(
          "*/api/activitypub/emoji-images/:id",
          () => new HttpResponse(null, { status: 404 }),
        ),
        http.get("*/api/home", () => {
          const e = { ...blobcat, image_id: "f".repeat(64) };
          return HttpResponse.json([
            {
              ...misskeyPost,
              emojis: [e],
              actor_emojis: [e],
              reactions: [{ reaction: e.name, emoji: e, count: 2, selected: false }],
            },
          ]);
        }),
        ...misskeyHandlers,
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getAllByText(":blobcat:").length).toBeGreaterThanOrEqual(2));
    await expect(canvas.queryByRole("img", { name: ":blobcat:" })).not.toBeInTheDocument();
  },
};
