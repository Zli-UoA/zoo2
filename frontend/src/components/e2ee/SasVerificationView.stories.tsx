import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, within } from "storybook/test";
import type { SasView } from "@zoo/crypto-wasm";

import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { SasEntry } from "../../lib/e2ee/sasController";
import { QrCodeImage } from "./QrCodeImage";
import { QrCodeScanner } from "./QrCodeScanner";
import { SasVerificationView, type SasVerificationHandlers } from "./SasVerificationView";

const handlers = (): SasVerificationHandlers => ({
  onAccept: fn(),
  onCancel: fn(),
  onMatch: fn(),
  onMismatch: fn(),
  onRetrySave: fn(),
  onClose: fn(),
  onStartSas: fn(),
  onScanQr: fn(async () => "retry" as const),
  onQrConfirm: fn(),
});

const TXN = "6f1c2b3a-4d5e-4f60-8a7b-9c0d1e2f3a4b";

/** 検証の QR のバイト列（Matrix の形。鍵と秘密は固定のダミー）。 */
const qrCode: number[] = [
  ...Array.from("MATRIX", (c) => c.charCodeAt(0)),
  0x02,
  0x00,
  0x00,
  TXN.length,
  ...Array.from(TXN, (c) => c.charCodeAt(0)),
  ...Array.from({ length: 64 }, (_, i) => (i * 37 + 11) & 0xff),
  ...Array.from({ length: 16 }, (_, i) => (i * 101 + 7) & 0xff),
];

const view: SasView = {
  transaction_id: TXN,
  their_user_id: "00000000-0000-4000-8000-0000000000b1",
  their_device_id: "2b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718",
  we_requested: true,
  started_at: 1_800_000_000_000,
  phase: "compare",
  // 同じ絵文字が 2 回出ることもある。
  emoji: [0, 23, 47, 47, 54, 12, 63],
  cancel: null,
  verified_master_key: null,
  method: "sas",
  qr_code: null,
  can_scan_qr_code: false,
  can_start_sas: false,
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

/** 遅延読み込みする QR の部品単体。生成したバイト列と、カメラを使えない状態の表示を確かめる。 */
export const QrParts: Story = {
  render: () => (
    <div className="flex flex-wrap items-start gap-6">
      <QrCodeImage bytes={qrCode} label="検証の QR コード" />
      <QrCodeScanner initialState="unsupported" onScan={fn(async () => "retry" as const)} />
    </div>
  ),
};

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

/** 要求を送った。QR を先に出し、その下に「絵文字で検証する」を出す（相手の受け入れを待つあいだも）。 */
export const Requested: Story = {
  args: {
    entry: entry({
      phase: "requested",
      their_device_id: null,
      emoji: null,
      method: null,
      qr_code: qrCode,
      can_start_sas: true,
    }),
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args }) => {
    await expect(await screen.findByRole("img", { name: "検証の QR コード" })).toBeInTheDocument();
    // 相手が受け入れるまでは、相手の QR は読み取れない。
    await expect(screen.queryByRole("button", { name: "相手の QR を読み取る" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "絵文字で検証する" }));
    await expect(args.handlers.onStartSas).toHaveBeenCalled();
  }),
};

/** 絵文字を選んだあと、相手が受け入れるのを待っている。 */
export const RequestedEmoji: Story = {
  args: {
    entry: entry({ phase: "requested", their_device_id: null, emoji: null, method: null }),
  },
};

/** 双方が受け入れた。QR を出し、相手の QR を読み取るか、絵文字で検証するかを選ぶ。 */
export const ChooseMethod: Story = {
  args: {
    entry: entry({
      phase: "choose_method",
      emoji: null,
      method: null,
      qr_code: qrCode,
      can_scan_qr_code: true,
      can_start_sas: true,
    }),
    handlers: handlers(),
    scannerProps: { initialState: "scanning" },
  },
  play: playForImplementation(async ({ args }) => {
    await expect(await screen.findByRole("img", { name: "検証の QR コード" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "相手の QR を読み取る" }));
    await expect(await screen.findByLabelText("カメラの映像")).toBeInTheDocument();
    await expect(screen.queryByRole("img", { name: "検証の QR コード" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "自分の QR を出す" }));
    await expect(await screen.findByRole("img", { name: "検証の QR コード" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "絵文字で検証する" }));
    await expect(args.handlers.onStartSas).toHaveBeenCalled();
  }),
};

/** カメラの使用が許可されなかった。絵文字を勧める。 */
export const CameraDenied: Story = {
  args: {
    entry: entry({
      phase: "choose_method",
      emoji: null,
      method: null,
      qr_code: qrCode,
      can_scan_qr_code: true,
      can_start_sas: true,
    }),
    scannerProps: { initialState: "denied" },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "相手の QR を読み取る" }));
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "カメラの使用が許可されませんでした",
    );
    await expect(screen.getByRole("button", { name: "絵文字で検証する" })).toBeEnabled();
  }),
};

/** 相手の QR を読み取った。相手の確認を待っている。 */
export const QrScanned: Story = {
  args: { entry: entry({ phase: "qr_scanned", emoji: null, method: "qr" }) },
};

/** 相手がこの端末の QR を読み取った。相手の端末で読み取ったかを聞く。 */
export const QrConfirm: Story = {
  args: { entry: entry({ phase: "qr_confirm", emoji: null, method: "qr" }), handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await expect(
      await screen.findByText("伊藤さくら さんが QR コードを読み取りましたか？"),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "読み取った" }));
    await expect(args.handlers.onQrConfirm).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "読み取っていない" }));
    await expect(args.handlers.onCancel).toHaveBeenCalled();
  }),
};

/** QR を読み取って確かめた。保存するには、ログインし直す（読み取りはクリックではないため）。 */
export const QrNeedsLogin: Story = {
  args: {
    entry: entry({ phase: "done", emoji: null, method: "qr" }, { save: "login" }),
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(await screen.findByRole("button", { name: "ログインして保存" }));
    await expect(args.handlers.onRetrySave).toHaveBeenCalled();
  }),
};

/** QR の鍵が、サーバーから届いた鍵と合わなかった。 */
export const QrKeyMismatch: Story = {
  args: {
    entry: entry({
      phase: "cancelled",
      emoji: null,
      method: "qr",
      cancel: { code: "m.key_mismatch", by_us: true },
    }),
  },
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
