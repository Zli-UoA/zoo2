import { searchDemoQuery, longSearchDemoQuery } from "../../../.storybook/preference-fixtures";
import { preferenceSurfaces } from "../../../.storybook/preference-stories";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";
import { delay, http, HttpResponse } from "msw";
import { SearchPreferencesProvider } from "./SearchPreferences";
import { SearchEngineSelect } from "./SearchEngineSelect";
import { MfmContent } from "../chat/messages/MfmContent";
import { SEARCH_ENGINES, type SearchEngine } from "../../lib/searchEngine";

const meta = {
  title: "Features/Preferences/MfmSearch",
  component: SearchEngineSelect,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <SearchPreferencesProvider>
        <div className={preferenceSurfaces["Features/Preferences/MfmSearch"]}>
          <Story />
          <div className="mfm-body text-sm">
            <MfmContent content={`${searchDemoQuery} 検索`} />
          </div>
        </div>
      </SearchPreferencesProvider>
    ),
  ],
} satisfies Meta<typeof SearchEngineSelect>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Google: Story = {
  play: playForImplementation(async () => {
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "検索エンジン" })).toBeEnabled(),
    );
    await expect(screen.getByRole("textbox", { name: "検索語" })).toHaveValue("猫 café & zoo");
    await expect(screen.getByRole("textbox")).toHaveAttribute("readonly");
    await expect(screen.getByRole("textbox").closest("form")!).toHaveAttribute(
      "action",
      SEARCH_ENGINES.google.action,
    );
  }),
};
const selected = (engine: SearchEngine): Story => ({
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/users/me/preferences", () => HttpResponse.json({ search_engine: engine })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue(engine));
    await expect(screen.getByRole("textbox").closest("form")!).toHaveAttribute(
      "action",
      SEARCH_ENGINES[engine].action,
    );
    await expect(screen.getByRole("textbox")).toHaveAttribute(
      "name",
      SEARCH_ENGINES[engine].parameter,
    );
  }),
});
export const DuckDuckGo: Story = selected("duckduckgo");
export const Yandex: Story = selected("yandex");
export const Baidu: Story = selected("baidu");
export const ChangeEngine: Story = {
  play: playForImplementation(async () => {
    await waitFor(() => expect(screen.getByRole("combobox")).toBeEnabled());
    await userEvent.selectOptions(screen.getByRole("combobox"), "yandex");
    await waitFor(() =>
      expect(screen.getByRole("textbox").closest("form")!).toHaveAttribute(
        "action",
        SEARCH_ENGINES.yandex.action,
      ),
    );
    await expect(screen.getByRole("textbox")).toHaveAttribute("name", "text");
  }),
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/users/me/preferences", async () => {
          await delay("infinite");
          return HttpResponse.json({ search_engine: "google" });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("combobox")).toBeDisabled();
  }),
};
export const Saving: Story = {
  parameters: {
    msw: {
      handlers: [
        http.patch("*/api/users/me/preferences", async () => {
          await delay("infinite");
          return HttpResponse.json({ data: { search_engine: "yandex" }, txid: null });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await waitFor(() => expect(screen.getByRole("combobox")).toBeEnabled());
    await userEvent.selectOptions(screen.getByRole("combobox"), "yandex");
    await expect(await screen.findByRole("combobox")).toBeDisabled();
    await expect(screen.getByRole("combobox")).toHaveAttribute("aria-busy", "true");
  }),
};
export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/users/me/preferences", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "検索設定を読み込めませんでした。",
    );
    await expect(await screen.findByRole("combobox")).toBeDisabled();
    await expect(screen.getByRole("button", { name: "再試行" })).toBeEnabled();
  }),
};
export const SaveError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.patch("*/api/users/me/preferences", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await waitFor(() => expect(screen.getByRole("combobox")).toBeEnabled());
    await userEvent.selectOptions(screen.getByRole("combobox"), "baidu");
    await expect(await screen.findByRole("alert")).toHaveTextContent("保存できませんでした。");
    await expect(screen.getByRole("combobox")).toHaveValue("google");
    await expect(screen.getByRole("textbox").closest("form")!).toHaveAttribute(
      "action",
      SEARCH_ENGINES.google.action,
    );
  }),
};
export const LongQueryMobile: Story = {
  globals: { viewport: { value: "mobile" } },
  render: () => (
    <>
      <SearchEngineSelect />
      <div className="mfm-body text-sm">
        <MfmContent content={`${longSearchDemoQuery} 検索`} />
      </div>
    </>
  ),
  play: playForImplementation(async () => {
    const searches = (await screen.findAllByRole("textbox")).map((field) => field.closest("form")!);
    for (const search of searches) {
      await expect(search.scrollWidth).toBeLessThanOrEqual(search.clientWidth);
      await expect(search.getBoundingClientRect().right).toBeLessThanOrEqual(
        document.documentElement.clientWidth,
      );
    }
  }),
};
