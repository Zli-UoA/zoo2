import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { initialSipState } from "../../lib/sip/controller";
import type { SipLine } from "../../lib/sip/types";
import { SipPanelView } from "./SipPanelView";
const personal: SipLine = {
  id: "home",
  name: "自宅のひかり電話",
  shared: false,
  owner_user_id: "alice",
  websocket_path: "/sip/ws",
  sip_domain: "pbx.example",
  username: "alice",
  phone_number: "0312345678",
  member_ids: [],
  phones: [{ name: "自宅の電話機", extension: "101" }],
  can_manage: true,
  can_connect: true,
};
const shared: SipLine = {
  ...personal,
  id: "circle",
  name: "サークル部室",
  shared: true,
  owner_user_id: null,
  member_ids: ["alice", "bob"],
  phones: [{ name: "部室の電話機", extension: "201" }],
};
const meta = {
  title: "Features/Calls/SipPhone",
  component: SipPanelView,
  parameters: { a11y: { test: "error" } },
  args: {
    directory: { enabled: true, lines: [personal, shared] },
    state: initialSipState,
    loading: false,
    loadError: false,
    isAdmin: true,
    channelBusy: false,
    users: [
      { id: "alice", name: "Alice" },
      { id: "bob", name: "Bob" },
    ],
    onRefresh: fn(),
    onConnect: fn(),
    onDisconnect: fn(),
    onDial: fn(),
    onAnswer: fn(),
    onHangup: fn(),
    onMute: fn(),
    onDtmf: fn(),
    onSave: fn(async () => {}),
    onRemove: fn(async () => {}),
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SipPanelView>;
export default meta;
type Story = StoryObj<typeof meta>;
export const PersonalAndShared: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const c = within(canvasElement);
    await userEvent.selectOptions(c.getByRole("combobox", { name: "発着信に使う回線" }), "circle");
    await userEvent.click(c.getByRole("button", { name: "接続して待ち受け" }));
    await expect(args.onConnect).toHaveBeenCalledWith("circle");
  }),
};
export const Empty: Story = { args: { directory: { enabled: true, lines: [] } } };
export const Disabled: Story = { args: { directory: { enabled: false, lines: [] } } };
export const Loading: Story = { args: { loading: true } };
export const LoadError: Story = { args: { loadError: true } };
export const SharedMember: Story = {
  args: { isAdmin: false, directory: { enabled: true, lines: [{ ...shared, can_manage: false }] } },
  play: playForImplementation(async ({ canvasElement }) => {
    const c = within(canvasElement);
    await expect(c.queryByRole("button", { name: "編集" })).not.toBeInTheDocument();
    await userEvent.click(c.getByRole("button", { name: "回線を登録" }));
    await expect(c.queryByRole("option", { name: "サークル共有" })).not.toBeInTheDocument();
  }),
};
export const Incoming: Story = {
  args: { state: { ...initialSipState, lineId: "home", status: "incoming" } },
  play: playForImplementation(async ({ canvasElement, args }) => {
    await userEvent.click(within(canvasElement).getByRole("button", { name: "応答" }));
    await expect(args.onAnswer).toHaveBeenCalledOnce();
  }),
};
export const Active: Story = {
  args: {
    state: {
      ...initialSipState,
      lineId: "home",
      status: "active",
      peer: "0312345678",
      muted: true,
    },
  },
};
export const Failed: Story = { args: { state: { ...initialSipState, error: true } } };
export const ChannelBusy: Story = { args: { channelBusy: true } };
export const EditKeepsPassword: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getAllByRole("button", { name: "編集" })[0]);
    await expect(c.getByLabelText("SIPパスワード")).toHaveValue("");
    await userEvent.clear(c.getByLabelText("回線名"));
    await userEvent.type(c.getByLabelText("回線名"), "自宅の新しい名前");
    await userEvent.click(c.getByRole("button", { name: "保存" }));
    await expect(args.onSave).toHaveBeenCalledWith(
      "home",
      expect.objectContaining({ name: "自宅の新しい名前", password: "" }),
    );
  }),
};
export const SaveError: Story = {
  args: {
    onSave: fn(async () => {
      throw new Error("503");
    }),
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getAllByRole("button", { name: "編集" })[0]);
    await userEvent.click(c.getByRole("button", { name: "保存" }));
    await expect(c.getByRole("alert")).toHaveTextContent("保存できませんでした");
    await expect(c.getByRole("button", { name: "保存" })).toBeEnabled();
  }),
};
export const Mobile: Story = {
  globals: { viewport: { value: "mobile1", isRotated: false } },
  args: {
    directory: {
      enabled: true,
      lines: [{ ...personal, name: "とても長い名前の自宅にある電話回線と電話機の設定" }, shared],
    },
  },
};

export const RegisterSharedLine: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getByRole("button", { name: "回線を登録" }));
    await userEvent.type(c.getByLabelText("回線名"), "サークルの新しい回線");
    await userEvent.selectOptions(c.getByLabelText("所有区分"), "shared");
    await userEvent.type(c.getByLabelText("SIPドメイン"), "pbx.example");
    await userEvent.type(c.getByLabelText("SIPユーザー名"), "circle");
    await userEvent.type(c.getByLabelText("SIPパスワード"), "dummy-password");
    await userEvent.click(c.getByRole("checkbox", { name: "Bob" }));
    await userEvent.click(c.getByRole("button", { name: "電話機を追加" }));
    await userEvent.type(c.getByLabelText("電話機・拠点名"), "部室");
    await userEvent.type(c.getByLabelText("発信先番号"), "201");
    await userEvent.click(c.getByRole("button", { name: "保存" }));
    await expect(args.onSave).toHaveBeenCalledWith(
      null,
      expect.objectContaining({
        shared: true,
        member_ids: ["bob"],
        phones: [{ name: "部室", extension: "201" }],
      }),
    );
  }),
};
export const Saving: Story = {
  args: { onSave: fn(() => new Promise<void>(() => {})) },
  play: playForImplementation(async ({ canvasElement }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getAllByRole("button", { name: "編集" })[0]);
    await userEvent.click(c.getByRole("button", { name: "保存" }));
    await expect(c.getByRole("button", { name: "保存中…" })).toBeDisabled();
  }),
};
export const DeleteConfirmation: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getAllByRole("button", { name: "削除" })[0]);
    const dialog = within(document.body).getByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "削除" }));
    await expect(args.onRemove).toHaveBeenCalledWith("home");
  }),
};

export const RemoveUnavailableMember: Story = {
  args: { users: [{ id: "bob", name: "Bob" }] },
  play: playForImplementation(async ({ canvasElement, args }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getAllByRole("button", { name: "編集" })[1]);
    await userEvent.click(c.getByRole("checkbox", { name: "利用できないメンバー · alice" }));
    await userEvent.click(c.getByRole("button", { name: "保存" }));
    await expect(args.onSave).toHaveBeenCalledWith(
      "circle",
      expect.objectContaining({ member_ids: ["bob"] }),
    );
  }),
};
