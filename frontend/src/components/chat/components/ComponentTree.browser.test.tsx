import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";
import interactive from "../../../../../crates/zoo-components/fixtures/valid/message-interactive.json";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ComponentTree } from "./ComponentTree";
import { parseComponents } from "./parse";

const { api, collections } = vi.hoisted(() => ({
  api: { request: vi.fn(async () => ({})), get: vi.fn() },
  // ユーザーとチャンネルの collection がない（同期前・Storybook）状態
  collections: { config: { apiBaseUrl: "" } },
}));
vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => collections,
  useOptionalCollections: () => collections,
}));
vi.mock("../../../lib/api/client", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  createConfiguredApiClient: () => api,
}));

const doc = parseComponents(interactive.document)!;

describe("ComponentTree", () => {
  it("renders every interactive component without synced users or channels", async () => {
    render(
      <LocaleProvider>
        <ComponentTree document={doc} />
      </LocaleProvider>,
    );
    // アプリがない（Webhook と同じ）ので押せない
    expect(await screen.findByRole("button", { name: "Approve" })).toBeDisabled();
    // Label の外のピッカーにも名前がある（axe の label ルール）
    for (const name of ["日付", "時刻", "日時"])
      expect(screen.getByLabelText(name)).toBeInTheDocument();
  });

  it("asks for confirmation before sending and sends picker values only on submit", async () => {
    render(
      <LocaleProvider>
        <ComponentTree document={doc} appId="app" messageId="m1" channelId="c1" />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(api.request).not.toHaveBeenCalled();
    await userEvent.click(await screen.findByRole("button", { name: "Yes" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    expect(JSON.stringify(api.request.mock.calls[0])).toContain('"approve":"yes"');
  });
});
