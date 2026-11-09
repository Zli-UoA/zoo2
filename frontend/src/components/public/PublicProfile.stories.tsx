import { storyCollections } from "../../../.storybook/fixtures";
import { avatarUsers, userAvatarHandler, userAvatarUrl } from "../../../.storybook/avatar-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse, delay } from "msw";
import { expect, userEvent, within, waitFor } from "storybook/test";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import {
  publicProfile,
  longPublicProfile,
  imagePublicProfile,
} from "../../../.storybook/public-profile-fixtures";
import { homeImageHandler } from "../../../.storybook/home-fixtures";
import { PublicProfile } from "./PublicProfile";

const endpoint = "*/api/activitypub/users/alice/profile";
const meta = {
  title: "Pages/PublicProfile",
  component: PublicProfile,
  args: { userId: "alice" },
  parameters: {
    a11y: { test: "error" },
    msw: { handlers: [http.get(endpoint, () => HttpResponse.json(publicProfile))] },
  },
} satisfies Meta<typeof PublicProfile>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      await canvas.findByRole("heading", { level: 1, name: "Alice / アリス" }),
    ).toBeVisible();
    await expect(canvas.getByRole("textbox")).toHaveValue("@alice@zoo.example");
    await expect(canvas.getByText("太字", { exact: true })).toBeVisible();
    await expect(canvas.getByRole("link", { name: "過去の投稿" })).toHaveAttribute(
      "href",
      "/public/users/alice?before=p1",
    );
    await expect(canvasElement.querySelector("footer")).toBeNull();
  }),
};
export const Empty: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get(endpoint, () =>
          HttpResponse.json({ ...publicProfile, posts: [], next_cursor: null }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(
      await within(canvasElement).findByText("公開の投稿はまだありません。"),
    ).toBeVisible();
  }),
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get(endpoint, async () => {
          await delay("infinite");
          return HttpResponse.json(publicProfile);
        }),
      ],
    },
  },
};
let failures = 0;
export const LoadError: Story = {
  beforeEach: () => {
    failures = 0;
  },
  parameters: {
    msw: {
      handlers: [
        http.get(endpoint, () =>
          failures++ === 0
            ? new HttpResponse(null, { status: 500 })
            : HttpResponse.json(publicProfile),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("alert")).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: "更新" }));
    await expect(
      await canvas.findByRole("heading", { level: 1, name: "Alice / アリス" }),
    ).toBeVisible();
  }),
};
export const NotFound: Story = {
  parameters: {
    msw: { handlers: [http.get(endpoint, () => new HttpResponse(null, { status: 404 }))] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(
      await within(canvasElement).findByText("プロフィールが見つかりません。"),
    ).toBeVisible();
  }),
};
export const LongContent: Story = {
  parameters: {
    msw: { handlers: [http.get(endpoint, () => HttpResponse.json(longPublicProfile))] },
  },
};
export const Mobile: Story = { globals: { viewport: { value: "mobile", isRotated: false } } };
export const Pagination: Story = {
  args: { before: "p1" },
  parameters: {
    msw: {
      handlers: [
        http.get(endpoint, () =>
          HttpResponse.json({
            ...publicProfile,
            posts: [publicProfile.posts[1]],
            next_cursor: null,
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("link", { name: "最新の投稿" })).toHaveAttribute(
      "href",
      "/public/users/alice",
    );
    await expect(canvas.queryByRole("link", { name: "過去の投稿" })).toBeNull();
  }),
};

export const WithImages: Story = {
  parameters: {
    msw: {
      handlers: [http.get(endpoint, () => HttpResponse.json(imagePublicProfile)), homeImageHandler],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    for (const name of ["profile-photo.png", "image-only.png"]) {
      const image = await canvas.findByRole("img", { name });
      await expect(image).toBeVisible();
      await waitFor(() => expect((image as HTMLImageElement).naturalWidth).toBeGreaterThan(0));
    }
    await expect(canvas.getByText("公開プロフィールの画像投稿です。")).toBeVisible();
  }),
};

export const UserAvatar: Story = {
  parameters: {
    collections: { ...storyCollections, users: avatarUsers },
    msw: {
      handlers: [
        userAvatarHandler,
        http.get(endpoint, () =>
          HttpResponse.json({
            ...publicProfile,
            account: { ...publicProfile.account, avatar_url: userAvatarUrl },
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(async () => {
      const image = canvasElement.querySelector<HTMLImageElement>(`img[src="${userAvatarUrl}"]`);
      await expect(image?.naturalWidth).toBeGreaterThan(0);
    });
  }),
};

export const UserAvatarFallback: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get(userAvatarUrl, () => new HttpResponse(null, { status: 404 })),
        http.get(endpoint, () =>
          HttpResponse.json({
            ...publicProfile,
            account: { ...publicProfile.account, avatar_url: `${userAvatarUrl}?missing-public` },
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const avatar = await within(canvasElement).findByRole("img", { name: "Alice / アリス" });
    await waitFor(async () => await expect(avatar).toHaveTextContent("A/"));
    await expect(avatar.querySelector("img")).toBeNull();
  }),
};
