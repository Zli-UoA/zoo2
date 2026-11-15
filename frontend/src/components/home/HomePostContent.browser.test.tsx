import "../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeAll, expect, it } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
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
