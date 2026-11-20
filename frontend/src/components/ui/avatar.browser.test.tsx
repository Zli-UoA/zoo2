import { render, screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { expect, it, vi } from "vite-plus/test";

import { mockServer, startMockServer } from "../../test/browser-setup";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";

vi.mock("../../lib/electric/collections-context", () => ({
  useOptionalCollections: () => null,
}));

it("loads Slack avatars through the shared media proxy endpoint", async () => {
  await startMockServer();
  const slackUrl = "https://avatars.slack-edge.com/profile.png?size=72&v=2";
  const image = '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>';
  const endpoint = vi.fn(() => HttpResponse.text("/test-media-proxy"));
  const direct = vi.fn(() => HttpResponse.error());
  const proxied = vi.fn(({ request }: { request: Request }) => {
    expect(new URL(request.url).searchParams.get("url")).toBe(slackUrl);
    return HttpResponse.text(image, { headers: { "Content-Type": "image/svg+xml" } });
  });

  const summaly = vi.fn(() => HttpResponse.error());
  mockServer.use(
    http.get("/media-proxy-endpoint", endpoint),
    http.get("/summaly-endpoint", summaly),
    http.get("/test-media-proxy", proxied),
    http.get("https://avatars.slack-edge.com/profile.png", direct),
  );
  render(
    <>
      <Avatar>
        <AvatarImage src={slackUrl} alt="Alice" />
        <AvatarFallback>A</AvatarFallback>
      </Avatar>
      <Avatar>
        <AvatarImage src={slackUrl} alt="Alice in the member list" />
        <AvatarFallback>A</AvatarFallback>
      </Avatar>
    </>,
  );

  expect(screen.getAllByText("A")).toHaveLength(2);
  await waitFor(() => expect(screen.getAllByRole("img", { name: /^Alice/ })).toHaveLength(2));
  const avatars = screen.getAllByRole("img", { name: /^Alice/ });
  for (const avatar of avatars) {
    expect(avatar).toHaveAttribute("src", `/test-media-proxy?url=${encodeURIComponent(slackUrl)}`);
  }
  expect(proxied).toHaveBeenCalled();
  expect(endpoint).toHaveBeenCalledTimes(1);
  expect(direct).not.toHaveBeenCalled();
  expect(summaly).not.toHaveBeenCalled();
});
