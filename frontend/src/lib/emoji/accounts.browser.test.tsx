import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { DEFAULT_CONFIG } from "../config";
import { useAuthZAccounts } from "./accounts";
const fixture = vi.hoisted(() => ({
  config: {} as import("../config").AppConfig,
  available: true,
}));
vi.mock("../electric/collections-context", () => ({
  useOptionalCollections: () => (fixture.available ? fixture : null),
}));
vi.mock("../electric/collections", () => ({
  getCollections: () => {
    throw Error("fixture invalid config");
  },
}));
function Consumer({ id }: { id: string }) {
  const accounts = useAuthZAccounts();
  return <output aria-label={id}>{accounts.map((a) => a.display_name).join(",")}</output>;
}
beforeEach(async () => {
  fixture.available = true;
  fixture.config = {
    ...DEFAULT_CONFIG,
    apiBaseUrl: "https://accounts.fixture.test",
    devUserId: "fixture",
  };
  await startMockServer();
});
it("uses the authenticated directory, coalesces consumers, and clears profiles on failure", async () => {
  let calls = 0;
  mockServer.use(
    http.get("https://accounts.fixture.test/api/emoji-accounts", ({ request }) => {
      calls++;
      expect(request.headers.get("X-Dev-User")).toBe("fixture");
      return calls === 1
        ? HttpResponse.json({
            accounts: [{ username: "alice", display_name: "AuthZ name", avatar_url: null }],
          })
        : new HttpResponse(null, { status: 503 });
    }),
  );
  const view = render(
    <>
      <Consumer id="first" />
      <Consumer id="second" />
    </>,
  );
  await waitFor(() => expect(screen.getByLabelText("second")).toHaveTextContent("AuthZ name"));
  expect(calls).toBe(1);
  view.unmount();
  render(<Consumer id="next" />);
  await waitFor(() => expect(calls).toBe(2));
  await waitFor(() => expect(screen.getByLabelText("next").textContent).toBe(""));
});

it("a configuration failure leaves the optional directory empty without crashing its parent", async () => {
  fixture.available = false;
  render(<Consumer id="unconfigured" />);
  await waitFor(() => expect(screen.getByLabelText("unconfigured").textContent).toBe(""));
});
