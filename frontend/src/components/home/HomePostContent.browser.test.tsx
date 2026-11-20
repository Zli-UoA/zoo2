import "../../styles.css";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { beforeAll, expect, it } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { HomePostContent } from "./HomePostContent";

beforeAll(startMockServer);

it(
  "expands Home URLs without a CollectionsProvider and shares Summaly requests",
  { timeout: 30_000 },
  async () => {
    const url = "https://example.com/home-without-provider";
    let lookups = 0;
    let endpointReads = 0;
    mockServer.use(
      http.get("*/summaly-endpoint", () => {
        endpointReads++;
        return HttpResponse.text("https://home-summaly.example/url");
      }),
      http.get("*/media-proxy-endpoint", () => {
        endpointReads++;
        return HttpResponse.text("https://home-media.example/");
      }),
      http.get("https://home-summaly.example/url", ({ request }) => {
        lookups++;
        expect(new URL(request.url).searchParams.get("url")).toBe(url);
        expect(request.credentials).toBe("omit");
        return HttpResponse.json({
          title: "Homeのリンクプレビュー",
          description: "Summalyで取得した説明",
          sitename: "Example",
        });
      }),
    );
    // The production shell uses the default collections singleton, with no Provider.
    const view = render(
      <>
        <HomePostContent content={url} />
        <HomePostContent content={`引用へのコメント\n\n<${url}>`} />
        <HomePostContent content={`<${url}>`} />
      </>,
    );
    // プレビューの取得は並列で動くブラウザーテストの負荷で1秒を超えることがある。
    const cards = await screen.findAllByRole(
      "link",
      { name: /Homeのリンクプレビュー/ },
      { timeout: 10_000 },
    );
    expect(cards).toHaveLength(3);
    for (const card of cards) expect(card).toHaveAttribute("href", url);
    expect(screen.getAllByText("Summalyで取得した説明")).toHaveLength(3);
    expect(lookups).toBe(1);
    expect(endpointReads).toBe(2);
    view.unmount();
    render(<HomePostContent content={url} />);
    await screen.findByRole("link", { name: /Homeのリンクプレビュー/ }, { timeout: 10_000 });
    expect(lookups).toBe(1);
    expect(endpointReads).toBe(2);
  },
);

it("preserves Home links without a Provider when Summaly fails", { timeout: 30_000 }, async () => {
  const url = "https://example.com/home-unavailable-preview";
  let lookups = 0;
  mockServer.use(
    http.get("*/summaly-endpoint", () => HttpResponse.text("https://home-summaly.example/url")),
    http.get("*/media-proxy-endpoint", () => HttpResponse.text("https://home-media.example/")),
    http.get("https://home-summaly.example/url", () => {
      lookups++;
      return HttpResponse.json({ error: "not found" }, { status: 404 });
    }),
  );
  render(<HomePostContent content={`<${url}>`} />);
  const link = screen.getByRole("link", { name: url.replace("https://", "") });
  // Summaly の取得は並列で動くブラウザーテストの負荷で1秒を超えることがある。
  await waitFor(() => expect(lookups).toBe(1), { timeout: 10_000 });
  expect(link).toHaveAttribute("href", url);
  expect(screen.getAllByRole("link")).toHaveLength(1);
  expect(screen.queryByRole("link", { name: `${url} example.com` })).toBeNull();
});

it("renders native quote cards without fetching quoted URLs through Summaly", async () => {
  const uri = "https://misskey.example/notes/native-quote";
  const normal = "https://example.com/article-in-comment";
  const more = ["https://example.com/comment-link-two", "https://example.com/comment-link-three"];
  const lookups: string[] = [];
  mockServer.use(
    http.get("*/summaly-endpoint", () => HttpResponse.text("https://home-summaly.example/url")),
    http.get("*/media-proxy-endpoint", () => HttpResponse.text("https://home-media.example/")),
    http.get("https://home-summaly.example/url", ({ request }) => {
      lookups.push(new URL(request.url).searchParams.get("url")!);
      return HttpResponse.json({
        title:
          new URL(request.url).searchParams.get("url") === normal
            ? "通常リンクのプレビュー"
            : "ほかの通常リンク",
        description: "通常URLにはSummalyを使う",
      });
    }),
  );
  const post = {
    id: uri,
    display_name: "Bob",
    user_name: "bob@misskey.example",
    created_at: "2026-11-01T09:00:00Z",
    content: "引用元の**本文** https://example.com/inside-quote",
    emojis: [],
  };
  const view = render(
    <LocaleProvider>
      <HomePostContent
        content={`コメント ${normal} ${more.join(" ")} [引用先](${uri})\n\n<${uri}>`}
        quote={{ uri, post }}
      />
      <HomePostContent content={`<${uri}>`} quote={{ uri, post }} />
      <HomePostContent
        content="<https://misskey.example/notes/missing>"
        quote={{ uri: "https://misskey.example/notes/missing", post: null }}
      />
    </LocaleProvider>,
  );
  await screen.findByRole("link", { name: /通常リンクのプレビュー/ });
  expect(screen.getAllByRole("group", { name: "引用元の投稿" })).toHaveLength(3);
  expect(screen.getAllByText("Bob")).toHaveLength(2);
  expect(screen.getAllByText("本文", { selector: "strong" })).toHaveLength(2);
  expect(screen.getAllByText("@bob@misskey.example")).toHaveLength(2);
  expect(screen.getByText("引用元の投稿を表示できません")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "引用元を開く" })).toHaveAttribute(
    "href",
    "https://misskey.example/notes/missing",
  );
  expect(screen.getByRole("link", { name: "引用先" })).toHaveAttribute("href", uri);
  expect(screen.queryByText("misskey.example/notes/native-quote")).toBeNull();
  await waitFor(() => expect(lookups).toHaveLength(3));
  expect([...lookups].sort()).toEqual([normal, ...more].sort());
  for (const width of [280, 960]) {
    view.container.style.width = `${width}px`;
    for (const card of screen.getAllByRole("group", { name: "引用元の投稿" })) {
      expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(width);
      expect(card.scrollWidth).toBeLessThanOrEqual(card.clientWidth);
    }
  }
});

it("opens local quote authors and timestamps in Zoo instead of the federation JSON endpoint", async () => {
  const id = "local-quoted-post";
  const root = createRootRoute();
  const index = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => (
      <LocaleProvider>
        <HomePostContent
          content="<https://zoo.example/api/activitypub/notes/local-quoted-post>"
          quote={{
            uri: "https://zoo.example/api/activitypub/notes/local-quoted-post",
            post: {
              id,
              remote: false,
              content: "Local original",
              user_name: "local",
              display_name: "Local author",
              created_at: "2026-11-01T09:00:00Z",
              emojis: [],
            },
          }}
        />
      </LocaleProvider>
    ),
  });
  const detail = createRoute({ getParentRoute: () => root, path: "/home/posts/$postId" });
  const router = createRouter({
    routeTree: root.addChildren([index, detail]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  render(<RouterProvider router={router} />);
  await screen.findByText("Local original");
  const links = screen.getAllByRole("link");
  expect(links).toHaveLength(2);
  for (const link of links) {
    expect(link).toHaveAttribute("href", `/home/posts/${id}`);
    expect(link).not.toHaveAttribute("target", "_blank");
  }
});

it("does not link unavailable local quotes to an ActivityPub JSON endpoint", () => {
  const uri = "https://zoo.example/api/activitypub/notes/deleted";
  render(
    <LocaleProvider>
      <HomePostContent content={`<${uri}>`} quote={{ uri, local: true, post: null }} />
    </LocaleProvider>,
  );
  expect(screen.getByText("引用元の投稿を表示できません")).toBeInTheDocument();
  expect(screen.queryByRole("link")).toBeNull();
});

it("uses the shared media viewer for ActivityPub video, audio and PDF attachments", async () => {
  const view = render(
    <LocaleProvider>
      <HomePostContent
        content=""
        previewLinks={false}
        media={[
          { url: "https://peer.example/movie.webm", name: "Movie", media_type: "video/webm" },
          { url: "https://peer.example/music.ogg", name: "Music", media_type: "audio/ogg" },
          { url: "https://peer.example/report.pdf", name: "Report", media_type: "application/pdf" },
        ]}
      />
    </LocaleProvider>,
  );
  expect(view.container.querySelector("video")?.getAttribute("src")).toBe(
    "https://peer.example/movie.webm",
  );
  expect(view.container.querySelector("audio")?.getAttribute("src")).toBe(
    "https://peer.example/music.ogg",
  );
  expect(view.container.querySelector("iframe")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Reportをプレビュー" }));
  const dialog = await screen.findByRole("dialog");
  expect(dialog.querySelector("canvas")?.getAttribute("role")).toBe("img");
});
