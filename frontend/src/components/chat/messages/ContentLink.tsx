import { Link, useRouter } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { cn } from "cn";
import { ExternalLink, inlineLinkClassName } from "../../ui/external-link";
import { parseChannelLink, requestMessageJump, type ChannelLinkTarget } from "./messageLink";

/**
 * 本文中の URL が、いま開いているアプリのチャンネル・メッセージへのリンクなら、その移動先を返す。
 * ルーターの外（Storybook やテストなど）や、別の origin の URL では `null`。
 * その場合の呼び出し側は、今までどおり外部リンクとして描く。
 */
export function useChannelLinkTarget(href: string): ChannelLinkTarget | null {
  const router = useRouter({ warn: false });
  if (!router || typeof window === "undefined") return null;
  return parseChannelLink(href, window.location.origin);
}

/**
 * このアプリのチャンネル・メッセージへのリンク（{@link useChannelLinkTarget} が返した移動先）を、
 * 新しいタブを開かずに同じタブで移動するリンクとして描く。見た目は呼び出し側の `className` で決める。
 * ルーターの `Link` を使うので、Ctrl・⌘・Shift・Alt・中クリックは、ブラウザー既定の動作（別タブ・別ウィンドウ）のまま。
 */
export function ChannelLink({
  target,
  className,
  children,
}: {
  target: ChannelLinkTarget;
  className?: string;
  children: ReactNode;
}) {
  const message = target.search.message;
  return (
    <Link
      to="/channels/$channelId"
      params={{ channelId: target.channelId }}
      search={target.search}
      className={className}
      onClick={(event) => {
        // 移動先がいま表示中の対象メッセージと同じだと、URL が変わらず、一覧が再スクロールしない。
        // ブラウザー既定の動作に任せる押し方（別タブなど）では、この画面を動かさない。
        if (!message || event.button !== 0) return;
        if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
        requestMessageJump(message);
      }}
    >
      {children}
    </Link>
  );
}

/**
 * 本文中のリンク。このアプリのチャンネル・メッセージへのリンクは、新しいタブを開かずに同じタブで移動する。
 * それ以外の URL は `ExternalLink`（別タブ・`noopener noreferrer`）で開く。呼び出し元の URL 検証は維持する。
 */
export function ContentLink({
  href,
  className,
  children,
}: {
  href: string;
  className?: string;
  children: ReactNode;
}) {
  const target = useChannelLinkTarget(href);
  if (!target) {
    return (
      <ExternalLink href={href} className={className}>
        {children}
      </ExternalLink>
    );
  }
  return (
    <ChannelLink target={target} className={cn(inlineLinkClassName, className)}>
      {children}
    </ChannelLink>
  );
}
