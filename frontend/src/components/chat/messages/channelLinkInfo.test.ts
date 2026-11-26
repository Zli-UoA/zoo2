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
    card: null,
    components: null,
    app_blocks: null,
    ciphertext: null,
    ...overrides,
  };
}

const ciphertext = {
  algorithm: "m.megolm.v1.aes-sha2" as const,
  sender_key: "key",
  device_id: "device",
  session_id: "session",
  ciphertext: "AwgA",
};
const hiddenComponents = JSON.stringify({
  flags: 1 << 15,
  components: [{ type: 10, content: "部品" }],
});
const card = JSON.stringify({
  title: "デプロイ完了",
  description: "",
  url: "https://example.com",
  source: "CI",
  context: "",
  author: "",
  badge: "",
});

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
          kind: "text",
          excerpt: "こんにちは リンク",
          attachmentCount: 0,
          forwarded: false,
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

describe("暗号化の判定（格下げ対策）", () => {
  const resolve = (
    channel: LinkedChannel,
    rows: ReturnType<typeof row>[],
    treatAsEncrypted = false,
  ) =>
    resolveChannelLinkInfo({
      directory: directory([channel]),
      target: messageTarget(channel.id, "m1"),
      messages: { rows, loading: false },
      treatAsEncrypted,
    });

  it("暗号文のある行は、チャンネルが `encrypted = false` でも、本文を出さずに暗号化として扱う", () => {
    // 暗号化メッセージの `content` は空。サーバーが平文の `content` を差し込んでも、出さない。
    const info = resolve(general, [row({ ciphertext, content: "差し込まれた平文" })]);
    expect(info.status === "message" && info.message.kind).toBe("encrypted");
    expect(info.status === "message" && info.message.excerpt).toBe("");
    expect(JSON.stringify(info)).not.toContain("差し込まれた平文");
  });

  it("暗号文のない行も、同じチャンネルの暗号文のある行とは別に判定する", () => {
    const info = resolve(general, [row({ ciphertext: null })]);
    expect(info.status === "message" && info.message.kind).toBe("text");
  });

  it("暗号化チャンネルだったのに `encrypted = false` が配られた（格下げ）チャンネルは、暗号化として扱う", () => {
    const info = resolve(general, [row({ content: "差し込まれた平文" })], true);
    expect(info).toMatchObject({ status: "message", channel: { encrypted: true } });
    expect(info.status === "message" && info.message.kind).toBe("encrypted");
    expect(JSON.stringify(info)).not.toContain("差し込まれた平文");
  });

  it("格下げの判定は、チャンネルのリンクにも効く（暗号化チャンネルとして示す）", () => {
    const info = resolveChannelLinkInfo({
      directory: directory([general]),
      target: channelTarget("c-general"),
      messages: undefined,
      treatAsEncrypted: true,
    });
    expect(info).toMatchObject({ status: "channel", channel: { encrypted: true } });
  });

  it("一覧のチャンネルの情報は、書き換えない", () => {
    const dir = directory([general]);
    resolveChannelLinkInfo({
      directory: dir,
      target: channelTarget("c-general"),
      messages: undefined,
      treatAsEncrypted: true,
    });
    expect(dir.byId.get("c-general")?.encrypted).toBe(false);
  });
});

describe("暗号化かどうかを確かめている間", () => {
  const checking = (channel: LinkedChannel, overrides: Record<string, unknown> = {}) =>
    resolveChannelLinkInfo({
      directory: directory([channel]),
      target: messageTarget(channel.id, "m1"),
      messages: { rows: [row(overrides)], loading: false },
      checkingEncryption: true,
    });

  it("平文の本文は出さず、読み込み中とする", () => {
    const info = checking(general, { content: "確認前の本文" });
    expect(info).toEqual({ status: "loading" });
    expect(JSON.stringify(info)).not.toContain("確認前の本文");
  });

  it("本文を出さないと決まっている行（暗号文の行）は、待たずに暗号化と出す", () => {
    const info = checking(general, { ciphertext, content: "" });
    expect(info.status === "message" && info.message.kind).toBe("encrypted");
  });

  it("本文を出さないと決まっている行（連携カード）は、待たずに App のメッセージと出す", () => {
    const info = checking(general, { card });
    expect(info.status === "message" && info.message.kind).toBe("app");
  });

  it("サーバーが暗号化と配るチャンネルは、待たずに暗号化と出す", () => {
    const info = checking(sealed);
    expect(info.status === "message" && info.message.kind).toBe("encrypted");
  });

  it("見つからないメッセージは、待たずに「表示できません」とする", () => {
    expect(checking(general, { id: "other" })).toEqual({
      status: "unavailable",
      subject: "message",
    });
  });

  it("チャンネルだけを指すリンクは、本文がないので、待たずに出す", () => {
    expect(
      resolveChannelLinkInfo({
        directory: directory([general]),
        target: channelTarget("c-general"),
        messages: undefined,
        checkingEncryption: true,
      }).status,
    ).toBe("channel");
  });
});

describe("連携カード・部品の投稿", () => {
  const kindOf = (overrides: Record<string, unknown>) => {
    const info = resolveChannelLinkInfo({
      directory: directory([general]),
      target: messageTarget("c-general", "m1"),
      messages: { rows: [row(overrides)], loading: false },
    });
    return info.status === "message" ? info.message : null;
  };

  it("本文を隠す部品の投稿は、`content` を出さず、`app` として扱う", () => {
    const message = kindOf({ components: hiddenComponents, content: "代わりの本文" });
    expect(message?.kind).toBe("app");
    expect(message?.excerpt).toBe("");
  });

  it("App のカードのブロックの投稿は、`content` を出さない", () => {
    const blocks = JSON.stringify([{ type: "card", card: JSON.parse(card) }]);
    expect(kindOf({ app_blocks: blocks, content: "代わりの本文" })?.kind).toBe("app");
  });

  it("連携カードの投稿は、`content` を出さない", () => {
    expect(kindOf({ card, content: "代わりの本文" })?.kind).toBe("app");
  });

  it("本文を隠さない部品や、読めないカードは、本文を出す", () => {
    expect(
      kindOf({
        components: JSON.stringify({ flags: 0, components: [{ type: 10, content: "部品" }] }),
      })?.kind,
    ).toBe("text");
    expect(kindOf({ card: "not json" })?.kind).toBe("text");
  });

  it("暗号化チャンネルでは、連携カードでも暗号化として扱う", () => {
    const info = resolveChannelLinkInfo({
      directory: directory([sealed]),
      target: messageTarget("c-sealed", "m1"),
      messages: { rows: [row({ card })], loading: false },
    });
    expect(info.status === "message" && info.message.kind).toBe("encrypted");
  });
});

describe("取得に失敗したリンク先", () => {
  const resolve = (
    messages: Parameters<typeof resolveChannelLinkInfo>[0]["messages"],
    extra: { checkingEncryption?: boolean; channels?: LinkedChannel[] } = {},
  ) =>
    resolveChannelLinkInfo({
      directory: directory(extra.channels ?? [general]),
      target: messageTarget("c-general", "m1"),
      messages,
      checkingEncryption: extra.checkingEncryption,
    });

  it.each([
    ["行がない", []],
    ["結果がない", undefined],
    ["別の ID の行だけ", [row({ id: "other" })]],
  ])("失敗したら、「見られない」とせず、失敗とする（%s）", (_name, rows) => {
    expect(resolve({ rows, loading: false, failed: true })).toEqual({ status: "failed" });
  });

  it("暗号化かどうかを確かめている間でも、取得の失敗は失敗とする", () => {
    expect(
      resolve({ rows: [], loading: false, failed: true }, { checkingEncryption: true }),
    ).toEqual({ status: "failed" });
  });

  it("取得済みの行があれば、あとで同期に失敗しても、表示する", () => {
    expect(resolve({ rows: [row()], loading: false, failed: true }).status).toBe("message");
  });

  it("削除済みと分かっている行は、失敗していても「見られない」", () => {
    expect(
      resolve({
        rows: [row({ deleted_at: "2026-11-23 08:00:00.000" })],
        loading: false,
        failed: true,
      }),
    ).toEqual({ status: "unavailable", subject: "message" });
  });

  it("一覧にないチャンネルは、取得の失敗より先に「見られない」", () => {
    expect(resolve({ rows: [], loading: false, failed: true }, { channels: [secret] })).toEqual({
      status: "unavailable",
      subject: "message",
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

  it.each([
    ["転送元がある", ["m-source"], true],
    ["転送元の配列が空", [], false],
    ["転送元がない（null）", null, false],
    ["列がない", undefined, false],
  ])("転送の投稿かを持つ（%s）", (_name, forwarded_message_ids, forwarded) => {
    expect(toLinkedMessage(row({ forwarded_message_ids }), general).forwarded).toBe(forwarded);
  });

  it("暗号化チャンネルの行は、転送の ID があっても転送の投稿として扱わない", () => {
    // 暗号化チャンネルでは転送を表示しない（docs/e2ee.md の「メッセージ転送（引用）」）。
    expect(toLinkedMessage(row({ forwarded_message_ids: ["m-source"] }), sealed).forwarded).toBe(
      false,
    );
  });
});
