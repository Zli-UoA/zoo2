import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  createSummalyClient,
  type SummalyClient,
  type LinkPreviewData as Preview,
} from "../../../lib/summaly";
import { resolvePublicConfig, type AppConfig } from "../../../lib/config";
import { useNearViewport } from "../../../hooks/useNearViewport";
import { useOptionalCollections } from "../../../lib/electric/collections-context";
import { proxyImageUrl } from "../../../lib/media-proxy";
import { Button } from "../../ui/button";
import { ChannelLinkEmbed } from "./ChannelLinkEmbed";
import { ChannelLink, useChannelLinkParser, useChannelLinkTarget } from "./ContentLink";
import { useChannelDirectory } from "./MfmChannels";
import type { ChannelLinkTarget } from "./messageLink";
import { previewUrls } from "./linkPreviews";
import { acquirePreview, cachedPreview, subscribePreview } from "./linkPreviewRequests";
import { subscribePreviewEndpoints, type PreviewEndpoints } from "../../../lib/preview-endpoints";

function LinkPreview({
  url,
  client,
  cacheKey,
  proxy,
  active,
  showFallback,
}: {
  url: string;
  client: SummalyClient | null;
  cacheKey: string;
  proxy: string;
  active: boolean;
  showFallback: boolean;
}) {
  const key = `${cacheKey}:${url}`;
  const cached = useMemo(() => cachedPreview(key), [key]);
  const [result, setResult] = useState<{ key: string; data: Preview | null } | null>(null);
  const [failedImage, setFailedImage] = useState<string | null>(null);
  const [failedIcon, setFailedIcon] = useState<string | null>(null);
  const channelLink = useChannelLinkTarget(url);
  useEffect(() => subscribePreview(key, (data) => setResult({ key, data })), [key]);
  useEffect(() => {
    if (!client || !active) return;
    let subscribed = true;
    const work = acquirePreview(client, key, url);
    void work.request.then((data) => {
      if (subscribed && data) setResult({ key, data });
    });
    return () => {
      subscribed = false;
      work.release();
    };
  }, [url, client, key, active]);
  const data = result?.key === key ? result.data : cached;
  const { title, description, image, siteName, icon } = data
    ? data
    : { title: url, description: "", image: null, siteName: new URL(url).hostname, icon: null };
  const src = proxyImageUrl(proxy, image);
  const iconSrc = proxyImageUrl(proxy, icon ?? null);
  if (!showFallback && !data) return null;
  const cardClassName =
    "my-2 flex h-32 w-full max-w-sm overflow-hidden rounded-lg border bg-card text-sm outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring";
  const card = (
    <>
      <span className="relative h-full w-24 shrink-0 bg-muted sm:w-28">
        {src && failedImage !== src ? (
          <img
            src={src}
            alt=""
            loading="lazy"
            referrerPolicy="no-referrer"
            className="absolute inset-0 size-full object-cover"
            onError={() => setFailedImage(src)}
          />
        ) : null}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-1 p-3">
        <span className="line-clamp-2 break-words font-semibold text-foreground">{title}</span>
        {description ? (
          <span className="line-clamp-2 break-words text-muted-foreground">{description}</span>
        ) : null}
        <span className="mt-1 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          {iconSrc && failedIcon !== iconSrc ? (
            <img
              src={iconSrc}
              alt=""
              loading="lazy"
              referrerPolicy="no-referrer"
              className="size-4 shrink-0 rounded-sm object-contain"
              onError={() => setFailedIcon(iconSrc)}
            />
          ) : null}
          <span className="truncate">{siteName || new URL(url).hostname}</span>
        </span>
      </span>
    </>
  );
  // このアプリのチャンネル・メッセージへのリンクは、新しいタブを開かずに同じタブで移動する。
  return channelLink ? (
    <ChannelLink target={channelLink} className={cardClassName}>
      {card}
    </ChannelLink>
  ) : (
    <a href={url} target="_blank" rel="noopener noreferrer" className={cardClassName}>
      {card}
    </a>
  );
}
interface PreviewProps {
  urls: string[];
  config: AppConfig;
  active?: boolean;
  showFallback?: boolean;
}

function ActiveLinkPreviews({ urls, config, active = true, showFallback = true }: PreviewProps) {
  const [sources, setSources] = useState<PreviewEndpoints | null>(null);
  const client = useMemo(
    () => (sources?.summaly ? createSummalyClient(sources.summaly) : null),
    [sources?.summaly],
  );
  useEffect(() => {
    if (active) return subscribePreviewEndpoints(config, setSources);
  }, [config, active]);
  return urls.map((url) => (
    <LinkPreview
      key={url}
      url={url}
      client={client}
      cacheKey={sources?.summaly ?? ""}
      proxy={sources?.media ?? ""}
      active={active}
      showFallback={showFallback}
    />
  ));
}

/** Some screens keep offscreen rows mounted (Home, threads, catch-up); only nearby rows fetch. */
function DeferredLinkPreviews({ children }: { children: (active: boolean) => ReactNode }) {
  const [ref, active] = useNearViewport<HTMLDivElement>();
  return (
    <div ref={ref} className="min-h-px">
      {children(active)}
    </div>
  );
}

/**
 * 外部の URL のカード。`manual` のときは、押したときだけ取得する（暗号化チャンネルのメッセージ。
 * カードを出すと URL を Summaly と画像プロクシに送るため）。
 */
function ExternalPreviews({
  urls,
  content,
  manual,
  ...props
}: PreviewProps & { content: string; manual: boolean }) {
  const { t } = useTranslation();
  // 押したときの本文にだけ許可する。編集で本文（URL）が変わったら、もう一度押すまで取得しない。
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  if (!urls.length) return null;
  if (manual && loadedFor !== content) {
    return (
      <Button
        type="button"
        variant="link"
        size="xs"
        className="h-auto min-h-8 -mx-2"
        onClick={() => setLoadedFor(content)}
      >
        {t("e2ee.loadPreviews")}
      </Button>
    );
  }
  return <ActiveLinkPreviews urls={urls} {...props} />;
}

export function LinkPreviews({
  content,
  excludeUrl,
  defer = false,
  showFallback = true,
  manualExternal = false,
}: {
  content: string;
  excludeUrl?: string;
  defer?: boolean;
  showFallback?: boolean;
  /**
   * 外部の URL のカードを、押したときだけ取得する（暗号化チャンネル）。このアプリのチャンネル・メッセージの
   * リンクは、外部へ送らないので、これに関わらず表示する。
   */
  manualExternal?: boolean;
}) {
  const collections = useOptionalCollections();
  // The production shell uses the collections singleton without a Provider.
  // Preview requests only need public endpoints, including on public profiles.
  const config = useMemo(() => collections?.config ?? resolvePublicConfig(), [collections?.config]);
  const urls = useMemo(() => previewUrls(content, excludeUrl), [content, excludeUrl]);
  const directory = useChannelDirectory();
  const parseChannelLink = useChannelLinkParser();
  // このアプリのチャンネル・メッセージのリンクは、Summaly に送らず、自分の持つデータで埋め込みにする。
  // チャンネルの一覧を持たない画面（公開プロフィールなど）では、今までどおり外部と同じカードにする。
  const { embeds, externals } = useMemo(() => {
    const embeds: { url: string; target: ChannelLinkTarget }[] = [];
    const externals: string[] = [];
    for (const url of urls) {
      const target = directory ? parseChannelLink(url) : null;
      if (target) embeds.push({ url, target });
      else externals.push(url);
    }
    return { embeds, externals };
  }, [urls, directory, parseChannelLink]);
  if (!urls.length) return null;
  const render = (active: boolean) => (
    <>
      {directory
        ? embeds.map(({ url, target }) => (
            <ChannelLinkEmbed
              key={url}
              target={target}
              directory={directory}
              active={active}
              showFallback={showFallback}
            />
          ))
        : null}
      <ExternalPreviews
        urls={externals}
        content={content}
        manual={manualExternal}
        config={config}
        active={active}
        showFallback={showFallback}
      />
    </>
  );
  return defer ? <DeferredLinkPreviews>{render}</DeferredLinkPreviews> : render(true);
}
