import { useEffect, useMemo, useState } from "react";
import {
  createSummalyClient,
  type SummalyClient,
  type LinkPreviewData as Preview,
} from "../../../lib/summaly";
import type { AppConfig } from "../../../lib/config";
import { useOptionalCollections } from "../../../lib/electric/collections-context";
import { previewUrls, proxyImageUrl } from "./linkPreviews";
import { acquirePreview } from "./linkPreviewRequests";
import { subscribePreviewEndpoints, type PreviewEndpoints } from "./linkPreviewEndpoints";

function LinkPreview({
  url,
  client,
  cacheKey,
  proxy,
}: {
  url: string;
  client: SummalyClient | null;
  cacheKey: string;
  proxy: string;
}) {
  const [result, setResult] = useState<{ url: string; data: Preview | null } | null>(null);
  const [failedImage, setFailedImage] = useState<string | null>(null);
  const [failedIcon, setFailedIcon] = useState<string | null>(null);
  useEffect(() => {
    if (!client) return;
    let active = true;
    const work = acquirePreview(client, `${cacheKey}:${url}`, url);
    void work.request.then((data) => {
      if (active) setResult({ url, data });
    });
    return () => {
      active = false;
      work.release();
    };
  }, [url, client, cacheKey]);
  const { title, description, image, siteName, icon } =
    result?.url === url && result.data
      ? result.data
      : { title: url, description: "", image: null, siteName: new URL(url).hostname, icon: null };
  const src = proxyImageUrl(proxy, image);
  const iconSrc = proxyImageUrl(proxy, icon ?? null);
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      className="my-2 flex h-32 w-full max-w-sm overflow-hidden rounded-lg border bg-card text-sm outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring"
    >
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
    </a>
  );
}
function ActiveLinkPreviews({ urls, config }: { urls: string[]; config: AppConfig }) {
  const [sources, setSources] = useState<PreviewEndpoints | null>(null);
  const client = useMemo(
    () => (sources?.summaly ? createSummalyClient(sources.summaly) : null),
    [sources?.summaly],
  );
  useEffect(() => subscribePreviewEndpoints(config, setSources), [config]);
  return urls.map((url) => (
    <LinkPreview
      key={url}
      url={url}
      client={client}
      cacheKey={sources?.summaly ?? ""}
      proxy={sources?.media ?? ""}
    />
  ));
}

export function LinkPreviews({ content }: { content: string }) {
  const collections = useOptionalCollections();
  const urls = useMemo(() => previewUrls(content), [content]);
  return collections && urls.length ? (
    <ActiveLinkPreviews urls={urls} config={collections.config} />
  ) : null;
}
