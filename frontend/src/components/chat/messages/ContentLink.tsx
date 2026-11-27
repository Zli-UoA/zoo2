import { Link, useRouter } from "@tanstack/react-router";
import { useCallback, type ReactNode, type Ref } from "react";
import { cn } from "cn";
import { useNearViewport } from "../../../hooks/useNearViewport";
import { ExternalLink, inlineLinkClassName } from "../../ui/external-link";
import { useChannelLinkInfo } from "./channelLinkInfo";
import { useShortChannelLinkLabel } from "./channelLinkLabel";
import {
  parseChannelLink,
  requestChannelLinkRevealIfSameDestination,
  requestMessageJump,
  type ChannelLinkTarget,
} from "./messageLink";
import { useChannelDirectory, type ChannelDirectory } from "./MfmChannels";

/**
 * 本文中の URL が、いま開いているアプリのチャンネル・メッセージへのリンクかを調べる関数を返す。
 * 移動先を返し、ルーターの外（Storybook やテストなど）や、別の origin の URL では `null`。
 * その場合の呼び出し側は、今までどおり外部リンクとして描く。
 */
export function useChannelLinkParser(): (href: string) => ChannelLinkTarget | null {
  const router = useRouter({ warn: false });
  const hasRouter = Boolean(router);
  return useCallback(
    (href: string) =>
      hasRouter && typeof window !== "undefined"
        ? parseChannelLink(href, window.location.origin)
        : null,
    [hasRouter],
  );
}

/** {@link useChannelLinkParser} を 1 つの URL に使う。 */
export function useChannelLinkTarget(href: string): ChannelLinkTarget | null {
  return useChannelLinkParser()(href);
}

/**
 * このアプリのチャンネル・メッセージへのリンク（{@link useChannelLinkTarget} が返した移動先）を、
 * 新しいタブを開かずに同じタブで移動するリンクとして描く。見た目は呼び出し側の `className` で決める。
 * ルーターの `Link` を使うので、Ctrl・⌘・Shift・Alt・中クリックは、ブラウザー既定の動作（別タブ・別ウィンドウ）のまま。
 */
export function ChannelLink({
  target,
  className,
  anchorRef,
  children,
}: {
  target: ChannelLinkTarget;
  className?: string;
  /** リンクの要素。画面の近くにあるかの判定などに使う。 */
  anchorRef?: Ref<HTMLAnchorElement>;
  children: ReactNode;
}) {
  const router = useRouter({ warn: false });
  const message = target.search.message;
  const linked = message ?? target.search.thread;
  return (
    <Link
      ref={anchorRef}
      to="/channels/$channelId"
      params={{ channelId: target.channelId }}
      search={target.search}
      className={className}
      onClick={(event) => {
        // 移動先がいま表示中の対象メッセージと同じだと、URL が変わらず、一覧が再スクロールしない。
        // ブラウザー既定の動作に任せる押し方（別タブなど）では、この画面を動かさない。
        if (!linked || event.button !== 0) return;
        if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
        // 同じ URL でも一覧を対象へ戻せるよう、メッセージを指す通常クリックでは常に送る。
        if (message) requestMessageJump(message);
        // 移動先が同じときだけ、ノート表示中でもスレッド・メッセージへ戻す合図を送る。
        requestChannelLinkRevealIfSameDestination(
          router?.state.location.href,
          event.currentTarget.href,
        );
      }}
    >
      {children}
    </Link>
  );
}

/**
 * 行き先が分かったら、URL の代わりに短い表示（メッセージは「○○さん: #チャンネル名」、チャンネルは
 * 「#チャンネル名」）を出す同じタブのリンク。取得中や、見られない行き先は、URL のまま出す。
 * 見られない行き先の名前は出さない（存在を知らせない）。
 */
function ResolvedChannelLink({
  target,
  directory,
  className,
  children,
}: {
  target: ChannelLinkTarget;
  directory: ChannelDirectory;
  className?: string;
  children: ReactNode;
}) {
  // 行き先の取得は、カードと同じく、画面の近くに来てから始める（全行をマウントする画面で、リンクの数だけ
  // 別のチャンネルの取得が一斉に始まらないように）。
  const [ref, near] = useNearViewport<HTMLAnchorElement>();
  const { info } = useChannelLinkInfo(target, directory, near);
  const label = useShortChannelLinkLabel(info);
  return (
    <ChannelLink target={target} className={className} anchorRef={ref}>
      {label ?? children}
    </ChannelLink>
  );
}

/**
 * 本文中のリンク。このアプリのチャンネル・メッセージへのリンクは、新しいタブを開かずに同じタブで移動する。
 * それ以外の URL は `ExternalLink`（別タブ・`noopener noreferrer`）で開く。呼び出し元の URL 検証は維持する。
 *
 * `bare` は、リンクの文字が URL そのものであること（`[文字](URL)` の文字を書き換えない）。チャンネルの
 * 一覧を持つ画面では、行き先が分かったリンクを短い表示にする。
 */
export function ContentLink({
  href,
  className,
  bare = false,
  children,
}: {
  href: string;
  className?: string;
  bare?: boolean;
  children: ReactNode;
}) {
  const target = useChannelLinkTarget(href);
  const directory = useChannelDirectory();
  if (!target) {
    return (
      <ExternalLink href={href} className={className}>
        {children}
      </ExternalLink>
    );
  }
  const linkClassName = cn(inlineLinkClassName, className);
  if (bare && directory) {
    return (
      <ResolvedChannelLink target={target} directory={directory} className={linkClassName}>
        {children}
      </ResolvedChannelLink>
    );
  }
  return (
    <ChannelLink target={target} className={linkClassName}>
      {children}
    </ChannelLink>
  );
}
