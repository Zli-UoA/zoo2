import { useEffect, useMemo, useState } from "react";
import { ApiError } from "../../../lib/api/client";
import {
  createSummalyClient,
  type SummalyClient,
  type LinkPreviewData as Preview,
} from "../../../lib/summaly";
import type { AppConfig } from "../../../lib/config";
import { useOptionalCollections } from "../../../lib/electric/collections-context";
import { previewUrls, proxyImageUrl } from "./linkPreviews";

// Requests and bounded retries are shared across the timeline and thread pane.
const cache = new Map<string, { at: number; request: Promise<Preview | null> }>();
let activeRequests = 0;
const waiting: (() => void)[] = [];
async function queuedPreview(client: SummalyClient, url: string) {
  if (activeRequests >= 4) await new Promise<void>((resolve) => waiting.push(resolve));
  else activeRequests++;
  try {
    return await client.get(url, {
      signal: AbortSignal.timeout(12_000),
    });
  } finally {
    const next = waiting.shift();
    if (next) next();
    else activeRequests--;
  }
}
async function retryPreview(client: SummalyClient, url: string): Promise<Preview | null> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await queuedPreview(client, url);
    } catch (error) {
      const transient =
        (error instanceof ApiError && [408, 429, 502, 503, 504].includes(error.status)) ||
        error instanceof TypeError ||
        (error instanceof DOMException &&
          (error.name === "TimeoutError" || error.name === "AbortError"));
      if (!transient) return null;
      if (attempt >= 3) throw error;
      // Release the request slot before waiting; allow the server's 10s fetches to finish.
      await new Promise<void>((resolve) => setTimeout(resolve, 2_000 * 2 ** attempt));
    }
  }
}
function load(client: SummalyClient, key: string, url: string) {
  const previous = cache.get(key);
  if (previous && Date.now() - previous.at < 300_000) return previous.request;
  if (cache.size >= 256) cache.clear();
  const request = retryPreview(client, url).catch(() => {
    // A later mount can retry transient failures even after this retry budget is exhausted.
    if (cache.get(key)?.request === request) cache.delete(key);
    return null;
  });
  cache.set(key, { at: Date.now(), request });
  return request;
}
interface Endpoints {
  summaly: string;
  media: string;
}
let runtimeEndpoints: Promise<Endpoints> | undefined;
async function readEndpoint(path: string): Promise<string> {
  try {
    const response = await fetch(path, { cache: "no-store" });
    return response.ok && response.headers.get("content-type")?.startsWith("text/plain")
      ? (await response.text()).trim()
      : "";
  } catch {
    return "";
  }
}
function endpoints(config: AppConfig): Promise<Endpoints> {
  runtimeEndpoints ??= Promise.all([
    readEndpoint("/summaly-endpoint"),
    readEndpoint("/media-proxy-endpoint"),
  ]).then(([summaly, media]) => ({ summaly, media }));
  return runtimeEndpoints.then((value) => ({
    summaly: value.summaly || config.summalyUrl || "",
    media: value.media || config.mediaProxyUrl || "",
  }));
}
function LinkPreview({
  url,
  client,
  cacheKey,
  proxy,
}: {
  url: string;
  client: SummalyClient;
  cacheKey: string;
  proxy: string;
}) {
  const [result, setResult] = useState<{ url: string; data: Preview | null } | null>(null);
  const [failedImage, setFailedImage] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    void load(client, `${cacheKey}:${url}`, url).then((data) => {
      if (active) setResult({ url, data });
    });
    return () => {
      active = false;
    };
  }, [url, client, cacheKey]);
  if (result?.url !== url || !result.data) return null;
  const { title, description, image } = result.data;
  const src = proxyImageUrl(proxy, image);
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      className="my-2 flex max-w-xl overflow-hidden rounded-lg border bg-card text-sm shadow-sm outline-none hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring"
    >
      {src && failedImage !== src ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          referrerPolicy="no-referrer"
          className="h-28 w-28 shrink-0 object-cover"
          onError={() => setFailedImage(src)}
        />
      ) : null}
      <span className="flex min-w-0 flex-col gap-1 p-3">
        <span className="truncate text-xs text-muted-foreground">{new URL(url).hostname}</span>
        <span className="line-clamp-2 break-words font-semibold text-brand">{title}</span>
        {description ? (
          <span className="line-clamp-2 break-words text-muted-foreground">{description}</span>
        ) : null}
      </span>
    </a>
  );
}
function ActiveLinkPreviews({ urls, config }: { urls: string[]; config: AppConfig }) {
  const [sources, setSources] = useState<Endpoints | null>(null);
  const client = useMemo(
    () => (sources?.summaly ? createSummalyClient(sources.summaly) : null),
    [sources?.summaly],
  );
  useEffect(() => {
    let active = true;
    void endpoints(config).then((value) => {
      if (active) setSources(value);
    });
    return () => {
      active = false;
    };
  }, [config]);
  if (!client || !sources) return null;
  return urls.map((url) => (
    <LinkPreview
      key={url}
      url={url}
      client={client}
      cacheKey={sources.summaly}
      proxy={sources.media}
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
