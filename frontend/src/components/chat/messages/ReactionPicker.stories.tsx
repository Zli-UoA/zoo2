import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test";
import { useState } from "react";

import {
  customEmojiFixture,
  manyCustomEmojiFixtures,
} from "../../../../.storybook/preference-fixtures";

import { RECENT_EMOJIS_STORAGE_KEY } from "../../../lib/emoji/recents";
import { Button } from "../../ui/button";
import { ReactionPicker } from "./ReactionPicker";

const minimalKitchenCatalog = {
  emojis: [
    ["1f600", "😀", "grinning face"],
    ["1f431", "🐱", "cat face"],
  ],
  dates: ["20260101"],
  pairs: { "0_1": 0 },
};

function ReactionPickerDemo({ onSelect }: { onSelect: (emoji: string) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="flex min-h-96 items-center justify-center bg-background p-8">
      <Button onClick={() => setOpen(true)}>リアクションを追加</Button>
      <ReactionPicker open={open} onOpenChange={setOpen} onSelect={onSelect} />
    </div>
  );
}

const meta = {
  title: "Features/Chat/ReactionPicker",
  component: ReactionPicker,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    open: { control: "boolean" },
  },
  args: {
    open: true,
    onOpenChange: fn(),
    onSelect: fn(),
  },
  render: (args) => <ReactionPickerDemo onSelect={args.onSelect} />,
} satisfies Meta<typeof ReactionPicker>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", {}, { timeout: 5000 });
    // The first open fetches the emoji catalogue and mounts a virtual grid.
    // Wait for that grid rather than applying Testing Library's 1s default.
    const picker = within(dialog);
    const rows = await picker.findAllByRole("row", {}, { timeout: 5000 });
    await expect(rows.every((row) => Number(row.getAttribute("aria-rowindex")) >= 1)).toBe(true);
    const cells = await picker.findAllByRole("gridcell", {}, { timeout: 5000 });
    await expect(cells.every((cell) => Number(cell.getAttribute("aria-colindex")) >= 1)).toBe(true);
  }),
};

export const Kitchen: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("tab", { name: "Emoji Kitchen" }));
    await waitFor(
      () => expect(screen.getByRole("combobox", { name: "1つ目の絵文字" })).toBeVisible(),
      { timeout: 5000 },
    );
  }),
};

export const SearchEmpty: Story = {
  parameters: {
    // カスタム絵文字の取得に失敗すると「検索結果なし」ではなく再試行を出すため、空の一覧を返す。
    msw: { handlers: [http.get("*/api/emojis", () => HttpResponse.json([]))] },
  },
  play: playForImplementation(async () => {
    const search = await screen.findByPlaceholderText("絵文字を検索");
    await userEvent.type(search, "zzzzzzzz");
    await expect(
      await screen.findByText("一致する絵文字がありません", {}, { timeout: 5000 }),
    ).toBeVisible();
  }),
};

export const Recents: Story = {
  beforeEach: () => {
    const previous = localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY);
    localStorage.setItem(RECENT_EMOJIS_STORAGE_KEY, JSON.stringify(["🎉", "👍"]));
    return () => {
      if (previous === null) localStorage.removeItem(RECENT_EMOJIS_STORAGE_KEY);
      else localStorage.setItem(RECENT_EMOJIS_STORAGE_KEY, previous);
    };
  },
  play: playForImplementation(async ({ args }) => {
    const recent = await screen.findByRole("group", { name: "最近使った絵文字" });
    await userEvent.click(within(recent).getByRole("button", { name: "👍" }));
    await expect(args.onSelect).toHaveBeenCalledWith("👍");
    await userEvent.click(await screen.findByRole("button", { name: "リアクションを追加" }));
    const reopened = await screen.findByRole("group", { name: "最近使った絵文字" });
    await expect(within(reopened).getAllByRole("button")[0]).toHaveAccessibleName("👍");
  }),
};

export const KitchenLoading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get(/emoji-kitchen.*\.json/, async () => {
          await delay(1500);
          return HttpResponse.json(minimalKitchenCatalog);
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("tab", { name: "Emoji Kitchen" }));
  }),
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};

export const ManyCustomEmojis: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: { handlers: [http.get("*/api/emojis", () => HttpResponse.json(manyCustomEmojiFixtures))] },
  },
  play: playForImplementation(async ({ args }) => {
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("tab", { name: "カスタム絵文字" }));
    const last = await within(dialog).findByRole("button", { name: ":scroll_199:" });
    const viewport = within(dialog).getByRole("region", { name: "カスタム絵文字" });
    // 画像の寸法で折り返しと高さが変わるため、読み込み完了後にスクロールする。
    await Promise.all(Array.from(viewport.querySelectorAll("img"), (image) => image.decode()));
    await expect(viewport.clientHeight).toBeGreaterThan(0);
    await expect(viewport.scrollHeight).toBeGreaterThan(viewport.clientHeight);
    await expect(viewport.getBoundingClientRect().bottom).toBeLessThanOrEqual(
      dialog.getBoundingClientRect().bottom,
    );
    viewport.scrollTo({ top: viewport.scrollHeight });
    await waitFor(() => expect(viewport.scrollTop).toBeGreaterThan(0));
    // スクロールが滑らかに進む場合があるため、最後の絵文字が見えるまで待つ。
    await waitFor(() =>
      expect(last.getBoundingClientRect().bottom).toBeLessThanOrEqual(
        viewport.getBoundingClientRect().bottom,
      ),
    );
    // 開始アニメーション中は親ダイアログのopacityが0でも、入力欄はDOMから取得できる。
    await waitFor(() =>
      expect(within(dialog).getByRole("searchbox", { name: "絵文字を検索" })).toBeVisible(),
    );
    await userEvent.click(last);
    await expect(args.onSelect).toHaveBeenCalledWith(":scroll_199:");
  }),
};

const rankedSearchFixtures = [
  { ...customEmojiFixture, name: "xkusa", category: "Zoo", aliases: [] },
  { ...customEmojiFixture, name: "kusa_hayasu", category: "Zoo", aliases: [] },
  { ...customEmojiFixture, name: "kusa", category: "Zoo", aliases: [] },
  { ...customEmojiFixture, name: "kusaa", category: "Zoo", aliases: [] },
];

export const SearchExactFirst: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: { handlers: [http.get("*/api/emojis", () => HttpResponse.json(rankedSearchFixtures))] },
  },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("tab", { name: "カスタム絵文字" }));
    await within(dialog).findByRole("button", { name: ":kusa:" });
    await userEvent.type(within(dialog).getByRole("searchbox", { name: "絵文字を検索" }), "kusa");
    const region = within(dialog).getByRole("region", { name: "カスタム絵文字" });
    // 完全一致、前方一致（短い順）、部分一致の順に並ぶ。
    await waitFor(() =>
      expect(
        within(region)
          .getAllByRole("button")
          .map((button) => button.getAttribute("aria-label")),
      ).toEqual([":kusa:", ":kusaa:", ":kusa_hayasu:", ":xkusa:"]),
    );
    // カーソルを載せると、登録名をツールチップで表示する。
    await userEvent.hover(within(region).getByRole("button", { name: ":kusa:" }));
    await waitFor(() =>
      expect(
        screen.getByText(":kusa:", { selector: '[data-slot="tooltip-content"]' }),
      ).toBeVisible(),
    );
  }),
};

export const ManyCustomEmojisMobile: Story = {
  ...ManyCustomEmojis,
  globals: { viewport: { value: "mobile" } },
};
