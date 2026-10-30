import { useEffect, useMemo, useState } from "react";
import { ApiError, createConfiguredApiClient, type ApiClient } from "../../../lib/api/client";
import type { AppConfig } from "../../../lib/config";
import { useOptionalCollections } from "../../../lib/electric/collections-context";
import { previewUrls, proxyImageUrl } from "./linkPreviews";

interface Preview {
  title: string;
  description: string;
  image: string | null;
}
// Requests and bounded retries are shared across the timeline and thread pane.
const cache = new Map<string, { at: number; request: Promise<Preview | null> }>();
let activeRequests = 0;
const waiting: (() => void)[] = [];
async function queuedPreview(client: ApiClient, url: string) {
  if (activeRequests >= 4) await new Promise<void>((resolve) => waiting.push(resolve));
  else activeRequests++;
  try {
    return await client.get<Preview>(`/api/link-preview?${new URLSearchParams({ url })}`, {
      signal: AbortSignal.timeout(12_000),
    });
  } finally {
    const next = waiting.shift();
    if (next) next();
    else activeRequests--;
  }
}
async function retryPreview(client: ApiClient, url: string): Promise<Preview | null> {
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
function load(client: ApiClient, key: string, url: string) {
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
let runtimeEndpoint: Promise<string> | undefined;
function endpoint(fallback: string) {
  runtimeEndpoint ??= fetch("/media-proxy-endpoint")
    .then(async (r) =>
      r.ok && r.headers.get("content-type")?.startsWith("text/plain")
        ? (await r.text()).trim()
        : "",
    )
    .catch(() => "");
  return runtimeEndpoint.then((value) => value || fallback);
}
function LinkPreview({
  url,
  client,
  cacheKey,
  proxy,
}: {
  url: string;
  client: ApiClient;
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
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [proxy, setProxy] = useState(config.mediaProxyUrl ?? "");
  useEffect(() => {
    if (!urls.length) return;
    let active = true;
    void endpoint(config.mediaProxyUrl ?? "").then((value) => {
      if (active) setProxy(value);
    });
    return () => {
      active = false;
    };
  }, [config.mediaProxyUrl, urls.length]);
  return urls.map((url) => (
    <LinkPreview
      key={url}
      url={url}
      client={client}
      cacheKey={`${config.apiBaseUrl}:${config.devUserId ?? "oidc"}`}
      proxy={proxy}
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
