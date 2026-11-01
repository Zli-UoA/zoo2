import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
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
      <div className="h-[720px] bg-background">
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
    await userEvent.click(await canvas.findByRole("button", { name: "投稿を削除" }));
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
        http.post("*/api/activitypub/follows", () => {
          followed = true;
          return HttpResponse.json({ actor_id: "https://social.example/users/bob" });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByText("ActivityPub・チャンネル連携"));
    const account = canvas.getByRole("textbox", { name: "外部アカウントまたはActor URL" });
    await waitFor(() => expect(account).toBeEnabled());
    await userEvent.type(account, "@bob@social.example");
    await userEvent.click(canvas.getByRole("button", { name: /^フォロー$/ }));
    await expect(await canvas.findByText(/Bob @ social.example/)).toBeVisible();
  },
};

let savedPattern = "ltl-%";
export const ChannelRule: Story = {
  beforeEach: () => {
    savedPattern = "ltl-%";
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/home", () => HttpResponse.json(posts)),
        http.get("*/api/home/settings", () =>
          HttpResponse.json({ ltl_channel_pattern: savedPattern, can_manage: true }),
        ),
        http.put("*/api/home/settings", async ({ request }) => {
          const body = (await request.json()) as { ltl_channel_pattern: string };
          savedPattern = body.ltl_channel_pattern;
          return HttpResponse.json({ ltl_channel_pattern: savedPattern });
        }),
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByText("ActivityPub・チャンネル連携"));
    const field = await canvas.findByRole("textbox", { name: "LTLに流すチャンネル名のルール" });
    await userEvent.clear(field);
    await userEvent.type(field, "public-%");
    await userEvent.click(canvas.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(savedPattern).toBe("public-%"));
    await expect(field).toHaveValue("public-%");
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
    await userEvent.click(await canvas.findByRole("button", { name: "投稿を編集" }));
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
    await userEvent.click(canvas.getByText("ActivityPub・チャンネル連携"));
    await expect(await canvas.findByRole("alert")).toBeVisible();
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
    await openComposer(canvasElement);
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
    await expect(form.getByRole("button", { name: "連合なし" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
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
    await expect(form.getByRole("button", { name: "連合なし" })).toBeDisabled();
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
