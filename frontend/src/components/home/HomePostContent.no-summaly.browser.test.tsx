import { render, screen, waitFor } from "@testing-library/react";
import { beforeAll, expect, it } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { HomePostContent } from "./HomePostContent";

beforeAll(startMockServer);

it("keeps only the body link in Home when Summaly is not configured", async () => {
  let endpointReads = 0;
  mockServer.use(
    http.get("*/summaly-endpoint", () => {
      endpointReads++;
      return HttpResponse.text("");
    }),
    http.get("*/media-proxy-endpoint", () => {
      endpointReads++;
      return HttpResponse.text("");
    }),
  );
  const url = "https://example.com/unconfigured-summaly";
  render(<HomePostContent content={url} />);
  await waitFor(() => expect(endpointReads).toBe(2));
  expect(screen.getAllByRole("link")).toHaveLength(1);
  expect(screen.getByRole("link")).toHaveAttribute("href", url);
});
