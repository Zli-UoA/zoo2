import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vite-plus/test";
import { createApiClient } from "../../lib/api/client";
import { SEARCH_ENGINES, searchUrl } from "../../lib/searchEngine";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { SearchEngineSelect } from "./SearchEngineSelect";
import { PersonalSearchPreferences } from "./SearchPreferences";
import { MfmContent } from "../chat/messages/MfmContent";
import "../../styles.css";

function setup(engine = "google") {
  let stored = engine;
  let failLoad = false;
  let failSave = false;
  const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
    const saving = init?.method === "PATCH";
    if ((saving && failSave) || (!saving && failLoad))
      return new Response("error", { status: 500 });
    if (saving && typeof init?.body === "string") stored = JSON.parse(init.body).search_engine;
    const data = { search_engine: stored };
    return Response.json(saving ? { data, txid: null } : data);
  });
  const client = createApiClient({ fetch, getToken: async () => null });
  const tree = (id = "alice") => (
    <StrictMode>
      <LocaleProvider>
        <PersonalSearchPreferences key={id} client={client} enabled>
          <SearchEngineSelect />
          <div style={{ width: 240 }}>
            <MfmContent content="猫 café & zoo 検索" />
          </div>
        </PersonalSearchPreferences>
      </LocaleProvider>
    </StrictMode>
  );
  return {
    tree,
    fetch,
    stored: () => stored,
    failLoad: (value: boolean) => {
      failLoad = value;
    },
    failSave: (value: boolean) => {
      failSave = value;
    },
  };
}
it("persists all four engines, keeps the query readonly, and opens encoded queries on click and Enter", async () => {
  const state = setup();
  render(state.tree());
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  try {
    await waitFor(() => expect(screen.getByRole("combobox")).toBeEnabled());
    const query = screen.getByRole("textbox");
    expect(query).toHaveAttribute("readonly");
    await userEvent.type(query, "changed");
    expect(query).toHaveValue("猫 café & zoo");
    for (const engine of Object.keys(SEARCH_ENGINES) as (keyof typeof SEARCH_ENGINES)[]) {
      await userEvent.selectOptions(screen.getByRole("combobox"), engine);
      await waitFor(() => expect(screen.getByRole("combobox")).toBeEnabled());
      expect(state.stored()).toBe(engine);
      await userEvent.click(screen.getByRole("button", { name: /で検索$/ }));
      expect(open).toHaveBeenLastCalledWith(
        searchUrl(engine, "猫 café & zoo"),
        "_blank",
        "noopener,noreferrer",
      );
      expect(
        new URL(String(open.mock.lastCall?.[0])).searchParams.get(SEARCH_ENGINES[engine].parameter),
      ).toBe("猫 café & zoo");
    }
    await userEvent.click(query);
    await userEvent.keyboard("{Enter}");
    expect(open).toHaveBeenCalledTimes(5);
  } finally {
    open.mockRestore();
  }
});
it("loads saved preferences on remount and clears them when the account changes", async () => {
  const state = setup("yandex");
  const view = render(state.tree());
  await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("yandex"));
  await userEvent.selectOptions(screen.getByRole("combobox"), "duckduckgo");
  await waitFor(() => expect(screen.getByRole("combobox")).toBeEnabled());
  view.rerender(state.tree("alice-reload"));
  await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("duckduckgo"));
  state.failLoad(true);
  view.rerender(state.tree("bob"));
  await screen.findByRole("alert");
  expect(screen.getByRole("combobox")).toHaveValue("google");
});
it("retries a failed load and retains the previous engine after a failed save", async () => {
  const state = setup("baidu");
  state.failLoad(true);
  render(state.tree());
  await screen.findByRole("alert");
  expect(screen.getByRole("combobox")).toBeDisabled();
  state.failLoad(false);
  await userEvent.click(screen.getByRole("button", { name: "再試行" }));
  await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("baidu"));
  state.failSave(true);
  await userEvent.selectOptions(screen.getByRole("combobox"), "google");
  await screen.findByRole("alert");
  expect(screen.getByRole("combobox")).toHaveValue("baidu");
  state.failSave(false);
  await userEvent.selectOptions(screen.getByRole("combobox"), "google");
  await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("google"));
  expect(screen.queryByRole("alert")).toBeNull();
});
it("sizes text fields to their contents and keeps long searches within narrow messages", () => {
  render(
    <LocaleProvider>
      <div style={{ width: 240 }}>
        <MfmContent
          content={`猫 検索\n猫についての検索 検索\n${"長い検索語abcdefgh ".repeat(25)}検索`}
        />
      </div>
    </LocaleProvider>,
  );
  const fields = screen.getAllByRole("textbox");
  expect(fields).toHaveLength(3);
  expect(fields[0].getBoundingClientRect().width).toBeLessThan(
    fields[1].getBoundingClientRect().width,
  );
  for (const field of fields)
    expect(field.getBoundingClientRect().height).toBe(fields[0].getBoundingClientRect().height);
  for (const form of screen.getAllByRole("textbox").map((field) => field.closest("form")!))
    expect(form.scrollWidth).toBeLessThanOrEqual(form.clientWidth);
});
