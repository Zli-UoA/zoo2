import "../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeAll, expect, it } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { HomePostContent } from "./HomePostContent";

beforeAll(startMockServer);

it("expands Home URLs without a CollectionsProvider and shares Summaly requests", async () => {
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
  const cards = await screen.findAllByRole("link", { name: /Homeのリンクプレビュー/ });
  expect(cards).toHaveLength(3);
  for (const card of cards) expect(card).toHaveAttribute("href", url);
  expect(screen.getAllByText("Summalyで取得した説明")).toHaveLength(3);
  expect(lookups).toBe(1);
  expect(endpointReads).toBe(2);
  view.unmount();
  render(<HomePostContent content={url} />);
  await screen.findByRole("link", { name: /Homeのリンクプレビュー/ });
  expect(lookups).toBe(1);
  expect(endpointReads).toBe(2);
});

it("preserves Home links without a Provider when Summaly fails", async () => {
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
  await waitFor(() => expect(lookups).toBe(1));
  expect(link).toHaveAttribute("href", url);
  // The card falls back to the URL while the original body link stays usable.
  expect(await screen.findByRole("link", { name: `${url} example.com` })).toHaveAttribute(
    "href",
    url,
  );
});

it("renders native quote cards without fetching quoted URLs through Summaly", async () => {
  const uri = "https://misskey.example/notes/native-quote";
  const normal = "https://example.com/article-in-comment";
  const lookups: string[] = [];
  mockServer.use(
    http.get("https://home-summaly.example/url", ({ request }) => {
      lookups.push(new URL(request.url).searchParams.get("url")!);
      return HttpResponse.json({
        title: "通常リンクのプレビュー",
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
        content={`コメント ${normal} [引用先](${uri})\n\n<${uri}>`}
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
  expect(lookups).toEqual([normal]);
  for (const width of [280, 960]) {
    view.container.style.width = `${width}px`;
    for (const card of screen.getAllByRole("group", { name: "引用元の投稿" })) {
      expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(width);
      expect(card.scrollWidth).toBeLessThanOrEqual(card.clientWidth);
    }
  }
});
