import { defaultParseSearch } from "@tanstack/react-router";
import { describe, expect, it } from "vite-plus/test";

import { Route } from "../../../routes/_shell.channels.$channelId";
import { messageLinkTarget, messageLinkUrl } from "./messageLink";

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
