import { describe, expect, it } from "vite-plus/test";

import { resolveChannelLinkInfo, toLinkedMessage } from "./channelLinkInfo";
import type { ChannelDirectory, LinkedChannel } from "./MfmChannels";
import type { ChannelLinkTarget } from "./messageLink";

const general: LinkedChannel = {
  id: "c-general",
  name: "general",
  description: "全体",
  isPrivate: false,
  isDm: false,
  encrypted: false,
};
const secret: LinkedChannel = { ...general, id: "c-secret", name: "secret", isPrivate: true };
const sealed: LinkedChannel = { ...secret, id: "c-sealed", name: "sealed", encrypted: true };

function directory(channels: LinkedChannel[], ready = true): ChannelDirectory {
  return { byId: new Map(channels.map((channel) => [channel.id, channel])), ready };
}

function row(overrides: Record<string, unknown> = {}) {
  return {
    id: "m1",
    user_id: "u-bob",
    content: "**こんにちは** [リンク](https://example.com)",
    parent_id: null,
    created_at: "2026-11-23 07:30:00.000",
    display_name: "Bob",
    user_name: "bob",
    avatar_url: "https://example.com/bob.png",
    is_bot: false,
    deleted_at: null,
    display_data: undefined,
    ...overrides,
  };
}

const channelTarget = (channelId: string): ChannelLinkTarget => ({ channelId, search: {} });
const messageTarget = (channelId: string, message: string, thread?: string): ChannelLinkTarget => ({
  channelId,
  search: { message, ...(thread ? { thread } : {}) },
});

describe("resolveChannelLinkInfo", () => {
  describe("チャンネルのリンク", () => {
    it("見られるチャンネルなら、チャンネルを返す", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: channelTarget("c-general"),
          messages: undefined,
        }),
      ).toEqual({ status: "channel", channel: general });
    });

    it("一覧にないチャンネルは、一覧の同期が終わっていれば見られないとする", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: channelTarget("c-unknown"),
          messages: undefined,
        }),
      ).toEqual({ status: "unavailable", subject: "channel" });
    });

    it("一覧の同期が終わるまでは、ないとは決めない", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([], false),
          target: channelTarget("c-general"),
          messages: undefined,
        }),
      ).toEqual({ status: "loading" });
    });

    it("同期の途中でも、一覧にあるチャンネルはすぐ返す", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general], false),
          target: channelTarget("c-general"),
          messages: undefined,
        }).status,
      ).toBe("channel");
    });
  });

  describe("メッセージのリンク", () => {
    it("見られないチャンネルのメッセージは、取得結果があっても見られないとする", () => {
      // 非公開で参加していないチャンネルは、一覧に来ない。メッセージの有無で存在を漏らさない。
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-secret", "m1"),
          messages: { rows: [row()], loading: false },
        }),
      ).toEqual({ status: "unavailable", subject: "message" });
    });

    it("一覧の同期が終わるまでは、見られないとは決めない", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([], false),
          target: messageTarget("c-general", "m1"),
          messages: undefined,
        }),
      ).toEqual({ status: "loading" });
    });

    it("まだ取得を始めていない間は、読み込み中", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-general", "m1"),
          messages: undefined,
        }),
      ).toEqual({ status: "loading" });
    });

    it("取得中で、行がまだなければ、読み込み中", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-general", "m1"),
          messages: { rows: [], loading: true },
        }),
      ).toEqual({ status: "loading" });
    });

    it("取得が終わって行がなければ、見られない", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-general", "m1"),
          messages: { rows: [], loading: false },
        }),
      ).toEqual({ status: "unavailable", subject: "message" });
    });

    it("削除済みのメッセージは、見られない", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-general", "m1"),
          messages: {
            rows: [row({ deleted_at: "2026-11-24 00:00:00.000" })],
            loading: false,
          },
        }),
      ).toEqual({ status: "unavailable", subject: "message" });
    });

    it("別の ID の行が返っても、リンク先として使わない", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-general", "m1"),
          messages: { rows: [row({ id: "other" })], loading: false },
        }).status,
      ).toBe("unavailable");
    });

    it("再取得中でも、行がすでにあれば、そのまま表示する", () => {
      expect(
        resolveChannelLinkInfo({
          directory: directory([general]),
          target: messageTarget("c-general", "m1"),
          messages: { rows: [row()], loading: true },
        }).status,
      ).toBe("message");
    });

    it("見られるメッセージは、投稿者と本文の要約を返す", () => {
      const info = resolveChannelLinkInfo({
        directory: directory([general]),
        target: messageTarget("c-general", "m1"),
        messages: { rows: [row()], loading: false },
      });
      expect(info).toEqual({
        status: "message",
        channel: general,
        message: {
          id: "m1",
          parentId: null,
          authorName: "Bob",
          avatarUrl: "https://example.com/bob.png",
          isBot: false,
          createdAt: "2026-11-23 07:30:00.000",
          excerpt: "こんにちは リンク",
          attachmentCount: 0,
        },
      });
    });

    it("`?thread=` だけのリンクは、スレッドの親のメッセージを指す", () => {
      const info = resolveChannelLinkInfo({
        directory: directory([general]),
        target: { channelId: "c-general", search: { thread: "root" } },
        messages: { rows: [row({ id: "root" })], loading: false },
      });
      expect(info.status).toBe("message");
    });
  });

  describe("暗号化チャンネル", () => {
    it("本文を要約に含めない", () => {
      const info = resolveChannelLinkInfo({
        directory: directory([sealed]),
        target: messageTarget("c-sealed", "m1"),
        messages: { rows: [row({ content: "秘密の本文" })], loading: false },
      });
      expect(info).toMatchObject({ status: "message", channel: { encrypted: true } });
      expect(info.status === "message" && info.message.excerpt).toBe("");
      expect(JSON.stringify(info)).not.toContain("秘密の本文");
    });

    it("投稿者や日時は、本文ではないので出す", () => {
      const info = resolveChannelLinkInfo({
        directory: directory([sealed]),
        target: messageTarget("c-sealed", "m1"),
        messages: { rows: [row()], loading: false },
      });
      expect(info.status === "message" && info.message.authorName).toBe("Bob");
    });
  });
});

describe("toLinkedMessage", () => {
  it("返信は、スレッドの親を持つ", () => {
    expect(toLinkedMessage(row({ parent_id: "root" }), general).parentId).toBe("root");
  });

  it("表示名がなければユーザー名、それもなければユーザー ID を投稿者にする", () => {
    expect(toLinkedMessage(row({ display_name: "" }), general).authorName).toBe("bob");
    expect(toLinkedMessage(row({ display_name: "", user_name: null }), general).authorName).toBe(
      "u-bob",
    );
  });

  it("アイコンがなければ null", () => {
    expect(toLinkedMessage(row({ avatar_url: undefined }), general).avatarUrl).toBeNull();
  });

  it("Bot の投稿を区別する", () => {
    expect(toLinkedMessage(row({ is_bot: true }), general).isBot).toBe(true);
  });

  it("添付ファイルの数を数える", () => {
    const display_data = {
      emojis: {},
      attachments: [{ id: "a1" }, { id: "a2" }],
      reactions: [],
      reply_count: 0,
    };
    expect(toLinkedMessage(row({ display_data, content: "" }), general).attachmentCount).toBe(2);
  });

  it("非公開チャンネルでも、見られるなら本文を要約する", () => {
    expect(toLinkedMessage(row({ content: "参加者向け" }), secret).excerpt).toBe("参加者向け");
  });
});
