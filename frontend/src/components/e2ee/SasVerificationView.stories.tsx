import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, within } from "storybook/test";
import type { SasView } from "@zoo/crypto-wasm";

import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { SasEntry } from "../../lib/e2ee/sasController";
import { SasVerificationView, type SasVerificationHandlers } from "./SasVerificationView";

const handlers = (): SasVerificationHandlers => ({
  onAccept: fn(),
  onCancel: fn(),
  onMatch: fn(),
  onMismatch: fn(),
  onRetrySave: fn(),
  onClose: fn(),
});

const view: SasView = {
  transaction_id: "6f1c2b3a-4d5e-4f60-8a7b-9c0d1e2f3a4b",
  their_user_id: "00000000-0000-4000-8000-0000000000b1",
  their_device_id: "2b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718",
  we_requested: true,
  started_at: 1_800_000_000_000,
  phase: "compare",
  // 同じ絵文字が 2 回出ることもある。
  emoji: [0, 23, 47, 47, 54, 12, 63],
  cancel: null,
  verified_master_key: null,
};

const entry = (patch: Partial<SasView>, rest: Partial<Omit<SasEntry, "view">> = {}): SasEntry => ({
  view: { ...view, ...patch },
  busy: false,
  save: "idle",
  error: null,
  ...rest,
});

const meta = {
  title: "Features/E2EE/SasVerification",
  component: SasVerificationView,
  parameters: { a11y: { test: "error" } },
  args: { entry: entry({}), name: "伊藤さくら", handlers: handlers() },
} satisfies Meta<typeof SasVerificationView>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 相手から検証を求められた。受け入れるか断る。 */
export const Incoming: Story = {
  args: {
    entry: entry({ phase: "incoming", we_requested: false, emoji: null }),
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(await screen.findByRole("button", { name: "受け入れる" }));
    await expect(args.handlers.onAccept).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "断る" }));
    await expect(args.handlers.onCancel).toHaveBeenCalled();
  }),
};

/** 解錠していない端末で受け入れようとした。 */
export const IncomingNeedsUnlock: Story = {
  args: {
    entry: entry({ phase: "incoming", we_requested: false, emoji: null }, { error: "needsUnlock" }),
  },
};

/** 要求を送り、相手が受け入れるのを待っている。 */
export const Requested: Story = {
  args: { entry: entry({ phase: "requested", their_device_id: null, emoji: null }) },
};

/** 鍵を交換している。 */
export const Exchanging: Story = {
  args: { entry: entry({ phase: "exchanging", emoji: null }) },
};

/** 絵文字を見比べる。 */
export const Compare: Story = {
  args: { entry: entry({}), handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    const list = await screen.findByRole("list", { name: "検証の絵文字" });
    await expect(list.querySelectorAll("li")).toHaveLength(7);
    // 同じ絵文字は何度でも出る（3 番目と 4 番目がどちらも「鍵」）。
    const items = within(list).getAllByRole("listitem");
    await expect(within(items[2]).getByText("鍵")).toBeInTheDocument();
    await expect(within(items[3]).getByText("鍵")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "一致する" }));
    await expect(args.handlers.onMatch).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "一致しない" }));
    await expect(args.handlers.onMismatch).toHaveBeenCalled();
  }),
};

/** 名前の長い相手（タイトルと絵文字の折り返し）。 */
export const CompareLongName: Story = {
  args: {
    entry: entry({}),
    name: "とても長い表示名のユーザー（研究室の共有アカウント・二〇二六年度）",
  },
};

/** 一致すると答え、相手の確認を待っている。 */
export const Confirmed: Story = {
  args: { entry: entry({ phase: "confirmed" }) },
};

/** 双方が確かめた。署名とピンを保存している。 */
export const Saving: Story = {
  args: { entry: entry({ phase: "done" }, { save: "saving" }) },
};

/** 検証済みとして保存した。 */
export const Saved: Story = {
  args: { entry: entry({ phase: "done" }, { save: "saved" }), handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await expect(await screen.findByText("伊藤さくら さんを検証しました")).toBeInTheDocument();
    // ダイアログ右上の閉じるボタンも同じ名前なので、フッターのボタンを押す。
    const footer = screen
      .getAllByRole("button", { name: "閉じる" })
      .find((button) => button.closest('[data-slot="dialog-footer"]'));
    await expect(footer).toBeDefined();
    await userEvent.click(footer!);
    await expect(args.handlers.onClose).toHaveBeenCalled();
  }),
};

/** 再認証できず保存できなかった。再認証し直して保存する。 */
export const NeedsReauth: Story = {
  args: { entry: entry({ phase: "done" }, { save: "reauth" }), handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(await screen.findByRole("button", { name: "保存し直す" }));
    await expect(args.handlers.onRetrySave).toHaveBeenCalled();
  }),
};

/** 絵文字が一致しなかった（相手が答えた）。 */
export const MismatchedSas: Story = {
  args: {
    entry: entry({ phase: "cancelled", cancel: { code: "m.mismatched_sas", by_us: false } }),
  },
};

/** 10 分以内に終わらなかった。 */
export const TimedOut: Story = {
  args: {
    entry: entry({ phase: "cancelled", emoji: null, cancel: { code: "m.timeout", by_us: true } }),
  },
};

/** 相手に送れなかった。 */
export const SendFailed: Story = {
  args: { entry: entry({ phase: "requested", emoji: null }, { error: "sendFailed" }) },
};
