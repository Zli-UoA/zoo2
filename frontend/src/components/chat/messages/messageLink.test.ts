import { defaultParseSearch } from "@tanstack/react-router";
import { describe, expect, it } from "vite-plus/test";

import { Route } from "../../../routes/_shell.channels.$channelId";
import {
  linkedMessageId,
  messageLinkTarget,
  messageLinkUrl,
  parseChannelLink,
  sameChannelLinkTarget,
} from "./messageLink";

const origin = "https://zli.chat";

/** コピーしたリンクを開いたとき、チャンネル画面のルートが受け取る検索パラメーター。 */
function searchOf(url: string) {
  const validateSearch = Route.options.validateSearch as (search: Record<string, unknown>) => {
    message?: string;
    thread?: string;
  };
  return validateSearch(defaultParseSearch(new URL(url).search));
}

describe("messageLinkTarget", () => {
  it("指すのはメッセージだけで、スレッドは開かない", () => {
    expect(messageLinkTarget("c1", "m1", null)).toEqual({
      channelId: "c1",
      search: { message: "m1" },
    });
  });

  it("返信はスレッドを開いたうえで、その返信へ移動する", () => {
    expect(messageLinkTarget("c1", "m2", "root")).toEqual({
      channelId: "c1",
      search: { thread: "root", message: "m2" },
    });
  });
});

describe("messageLinkUrl", () => {
  it("チャンネル直下のメッセージへの絶対 URL を作る", () => {
    expect(messageLinkUrl(origin, "channel-1", "message-1", null)).toBe(
      "https://zli.chat/channels/channel-1?message=message-1",
    );
  });

  it("返信の URL には、開くスレッドのルートを含める", () => {
    expect(messageLinkUrl(origin, "channel-1", "reply-1", "root-1")).toBe(
      "https://zli.chat/channels/channel-1?thread=root-1&message=reply-1",
    );
  });

  it("開発環境の origin（ポート付き）もそのまま使う", () => {
    expect(messageLinkUrl("http://localhost:5173", "c", "m", null)).toBe(
      "http://localhost:5173/channels/c?message=m",
    );
  });

  it("ID の記号をエスケープする", () => {
    const url = messageLinkUrl(origin, "a/b?c", "m#1&x=y", null);
    expect(new URL(url).pathname).toBe("/channels/a%2Fb%3Fc");
    expect(searchOf(url)).toEqual({ message: "m#1&x=y", thread: undefined });
  });

  it("チャンネル画面のルートが、そのままメッセージ ID として読める", () => {
    expect(searchOf(messageLinkUrl(origin, "c", "m", null))).toEqual({
      message: "m",
      thread: undefined,
    });
    expect(searchOf(messageLinkUrl(origin, "c", "m", "root"))).toEqual({
      message: "m",
      thread: "root",
    });
  });

  it("数字だけの ID も文字列のまま読める", () => {
    // 引用符なしだと、ルーターが数値として読んで、メッセージを開けなくなる。
    expect(searchOf(messageLinkUrl(origin, "c", "12345", "67890"))).toEqual({
      message: "12345",
      thread: "67890",
    });
  });
});

describe("parseChannelLink", () => {
  it("messageLinkUrl が作ったリンクを、同じ移動先に戻す", () => {
    expect(parseChannelLink(messageLinkUrl(origin, "c1", "m1", null), origin)).toEqual({
      channelId: "c1",
      search: { message: "m1" },
    });
    expect(parseChannelLink(messageLinkUrl(origin, "c1", "m2", "root"), origin)).toEqual({
      channelId: "c1",
      search: { thread: "root", message: "m2" },
    });
  });

  it("エスケープした ID と、数字だけの ID も読み戻す", () => {
    expect(parseChannelLink(messageLinkUrl(origin, "a/b?c", "m#1&x=y", null), origin)).toEqual({
      channelId: "a/b?c",
      search: { message: "m#1&x=y" },
    });
    expect(parseChannelLink(messageLinkUrl(origin, "c", "12345", "67890"), origin)).toEqual({
      channelId: "c",
      search: { thread: "67890", message: "12345" },
    });
  });

  it("message のないチャンネルのリンクも、チャンネルそのものを指す移動先にする", () => {
    expect(parseChannelLink(`${origin}/channels/c1`, origin)).toEqual({
      channelId: "c1",
      search: {},
    });
    expect(parseChannelLink(`${origin}/channels/c1/`, origin)).toEqual({
      channelId: "c1",
      search: {},
    });
    expect(parseChannelLink(`${origin}/channels/c1?thread=root`, origin)).toEqual({
      channelId: "c1",
      search: { thread: "root" },
    });
  });

  it("ルートが読まない検索パラメーターと、ハッシュは無視する", () => {
    expect(parseChannelLink(`${origin}/channels/c1?utm_source=x&message=m1#top`, origin)).toEqual({
      channelId: "c1",
      search: { message: "m1" },
    });
    expect(parseChannelLink(`${origin}/channels/c1?message=`, origin)).toEqual({
      channelId: "c1",
      search: {},
    });
  });

  it.each([
    ["別のサイト", "https://example.com/channels/c1?message=m1"],
    ["同じホストの別ポート", "https://zli.chat:8443/channels/c1"],
    ["http の同じホスト", "http://zli.chat/channels/c1"],
    ["ユーザー情報付き", "https://user:pass@zli.chat/channels/c1"],
    ["チャンネルではないパス", "https://zli.chat/settings"],
    ["API のパス", "https://zli.chat/api/channels/c1"],
    ["チャンネル ID がない", "https://zli.chat/channels/"],
    ["ID の後ろに続くパス", "https://zli.chat/channels/c1/extra"],
    ["不正なエスケープ", "https://zli.chat/channels/%E0%A4%A"],
    ["URL ではない文字列", "not a url"],
  ])("%s は、アプリ内のリンクとして扱わない", (_name, href) => {
    expect(parseChannelLink(href, origin)).toBeNull();
  });
});

describe("sameChannelLinkTarget", () => {
  const target = { channelId: "c1", search: { message: "m1" } };

  it("チャンネル・メッセージ・スレッドが同じなら同じ移動先", () => {
    expect(sameChannelLinkTarget(target, { channelId: "c1", search: { message: "m1" } })).toBe(
      true,
    );
    expect(
      sameChannelLinkTarget(
        { channelId: "c1", search: { thread: "root", message: "m2" } },
        { channelId: "c1", search: { message: "m2", thread: "root" } },
      ),
    ).toBe(true);
  });

  it("チャンネル・メッセージ・スレッドのどれかが違えば別の移動先", () => {
    expect(sameChannelLinkTarget(target, { channelId: "c2", search: { message: "m1" } })).toBe(
      false,
    );
    expect(sameChannelLinkTarget(target, { channelId: "c1", search: { message: "m2" } })).toBe(
      false,
    );
    expect(
      sameChannelLinkTarget(target, { channelId: "c1", search: { message: "m1", thread: "root" } }),
    ).toBe(false);
    expect(
      sameChannelLinkTarget(
        { channelId: "c1", search: { thread: "root", message: "m1" } },
        { channelId: "c1", search: { thread: "other", message: "m1" } },
      ),
    ).toBe(false);
  });

  it("チャンネルだけの移動先は、メッセージやスレッド付きと区別する", () => {
    expect(
      sameChannelLinkTarget(
        { channelId: "c1", search: {} },
        { channelId: "c1", search: { message: "m1" } },
      ),
    ).toBe(false);
    expect(
      sameChannelLinkTarget(
        { channelId: "c1", search: { thread: "root" } },
        { channelId: "c1", search: {} },
      ),
    ).toBe(false);
  });
});

describe("linkedMessageId", () => {
  it("`message` があれば、それがリンク先のメッセージ", () => {
    expect(linkedMessageId({ channelId: "c1", search: { message: "m1" } })).toBe("m1");
  });

  it("返信へのリンクは、スレッドの親ではなく返信を指す", () => {
    expect(linkedMessageId({ channelId: "c1", search: { thread: "root", message: "m2" } })).toBe(
      "m2",
    );
  });

  it("`thread` だけのリンクは、スレッドの親を指す", () => {
    expect(linkedMessageId({ channelId: "c1", search: { thread: "root" } })).toBe("root");
  });

  it("チャンネルそのものへのリンクは、メッセージを指さない", () => {
    expect(linkedMessageId({ channelId: "c1", search: {} })).toBeNull();
  });
});
