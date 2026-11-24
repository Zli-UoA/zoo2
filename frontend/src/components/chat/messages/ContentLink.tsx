import { Link, useRouter } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { cn } from "cn";
import { ExternalLink, inlineLinkClassName } from "../../ui/external-link";
import { parseChannelLink, type ChannelLinkTarget } from "./messageLink";

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
 * 本文中のリンク。このアプリのチャンネル・メッセージへのリンクは、新しいタブを開かずに同じタブで移動する。
 * ルーターの `Link` を使うので、Ctrl・⌘・Shift・中クリックは、ブラウザー既定の動作（別タブ・別ウィンドウ）のまま。
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
    <Link
      to="/channels/$channelId"
      params={{ channelId: target.channelId }}
      search={target.search}
      className={cn(inlineLinkClassName, className)}
    >
      {children}
    </Link>
  );
}
