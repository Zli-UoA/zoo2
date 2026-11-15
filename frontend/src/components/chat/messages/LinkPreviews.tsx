import { useEffect, useMemo, useRef, useState } from "react";
import {
  createSummalyClient,
  type SummalyClient,
  type LinkPreviewData as Preview,
} from "../../../lib/summaly";
import { resolvePublicConfig, type AppConfig } from "../../../lib/config";
import { useOptionalCollections } from "../../../lib/electric/collections-context";
import { previewUrls, proxyImageUrl } from "./linkPreviews";
import { acquirePreview } from "./linkPreviewRequests";
import { subscribePreviewEndpoints, type PreviewEndpoints } from "./linkPreviewEndpoints";

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
  const [result, setResult] = useState<{ url: string; data: Preview | null } | null>(null);
  const [failedImage, setFailedImage] = useState<string | null>(null);
  const [failedIcon, setFailedIcon] = useState<string | null>(null);
  useEffect(() => {
    if (!client || !active) return;
    let subscribed = true;
    const work = acquirePreview(client, `${cacheKey}:${url}`, url);
    void work.request.then((data) => {
      if (subscribed) setResult({ url, data });
    });
    return () => {
      subscribed = false;
      work.release();
    };
  }, [url, client, cacheKey, active]);
  const { title, description, image, siteName, icon } =
    result?.url === url && result.data
      ? result.data
      : { title: url, description: "", image: null, siteName: new URL(url).hostname, icon: null };
  const src = proxyImageUrl(proxy, image);
  const iconSrc = proxyImageUrl(proxy, icon ?? null);
  if (!showFallback && !(result?.url === url && result.data)) return null;
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

/** Home and public profiles keep offscreen rows mounted; only nearby rows fetch. */
function DeferredLinkPreviews(props: PreviewProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [nearby, setNearby] = useState(false);
  const [visible, setVisible] = useState(
    () => typeof document !== "undefined" && document.visibilityState !== "hidden",
  );
  useEffect(() => {
    const update = () => setVisible(document.visibilityState !== "hidden");
    update();
    document.addEventListener("visibilitychange", update);
    if (typeof IntersectionObserver === "undefined") {
      setNearby(true);
      return () => document.removeEventListener("visibilitychange", update);
    }
    const observer = new IntersectionObserver(
      ([entry]) => setNearby(entry?.isIntersecting ?? false),
      { rootMargin: "200px 0px" },
    );
    if (ref.current) observer.observe(ref.current);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return (
    <div ref={ref} className="min-h-px">
      <ActiveLinkPreviews {...props} active={nearby && visible} />
    </div>
  );
}

export function LinkPreviews({
  content,
  excludeUrl,
  defer = false,
  showFallback = true,
}: {
  content: string;
  excludeUrl?: string;
  defer?: boolean;
  showFallback?: boolean;
}) {
  const collections = useOptionalCollections();
  // The production shell uses the collections singleton without a Provider.
  // Preview requests only need public endpoints, including on public profiles.
  const config = useMemo(() => collections?.config ?? resolvePublicConfig(), [collections?.config]);
  const urls = useMemo(() => previewUrls(content, excludeUrl), [content, excludeUrl]);
  if (!urls.length) return null;
  const props = { urls, config, showFallback };
  return defer ? <DeferredLinkPreviews {...props} /> : <ActiveLinkPreviews {...props} />;
}
