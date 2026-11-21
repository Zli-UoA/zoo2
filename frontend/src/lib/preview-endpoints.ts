import type { AppConfig } from "./config";

export interface PreviewEndpoints {
  summaly: string;
  media: string;
}

async function readEndpoint(path: string): Promise<string> {
  const response = await fetch(path, { cache: "no-store", signal: AbortSignal.timeout(5_000) });
  if (!response.ok) throw new Error("Preview endpoint configuration unavailable");
  // The Vite development server serves HTML here; use its build-time configuration.
  return response.headers.get("content-type")?.startsWith("text/plain")
    ? (await response.text()).trim()
    : "";
}

/** Share the runtime request and retry schedule for each endpoint within the tab. */
function runtimeEndpoint(path: string) {
  let request: Promise<string> | undefined;
  let value: string | undefined;
  const subscribers = new Set<(value: string) => void>();
  let pending = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let backoff = 2_000;

  function read() {
    request ??= readEndpoint(path)
      .then((endpoint) => (value = endpoint))
      .catch((error) => {
        request = undefined;
        throw error;
      });
    return request;
  }

  function stopRetrying() {
    clearTimeout(timer);
    timer = undefined;
    window.removeEventListener("online", refresh);
  }

  async function refresh() {
    if (pending || !subscribers.size) return;
    clearTimeout(timer);
    timer = undefined;
    pending = true;
    try {
      const endpoint = await read();
      for (const notify of subscribers) notify(endpoint);
      stopRetrying();
    } catch {
      for (const notify of subscribers) notify("");
      if (subscribers.size) {
        timer = setTimeout(refresh, backoff);
        backoff = Math.min(backoff * 2, 30_000);
      }
    } finally {
      pending = false;
    }
  }

  function subscribe(fallback: string, onChange: (value: string) => void) {
    if (value !== undefined) {
      onChange(value || fallback);
      return () => {};
    }
    const notify = (endpoint: string) => onChange(endpoint || fallback);
    subscribers.add(notify);
    if (subscribers.size === 1) {
      backoff = 2_000;
      window.addEventListener("online", refresh);
      void refresh();
    }
    return () => {
      subscribers.delete(notify);
      if (!subscribers.size) stopRetrying();
    };
  }

  return { read, subscribe };
}

const summalyEndpoint = runtimeEndpoint("/summaly-endpoint");
const mediaProxyEndpoint = runtimeEndpoint("/media-proxy-endpoint");

export async function previewEndpoints(config: AppConfig): Promise<PreviewEndpoints> {
  const [summaly, media] = await Promise.all([summalyEndpoint.read(), mediaProxyEndpoint.read()]);
  return {
    summaly: summaly || config.summalyUrl || "",
    media: media || config.mediaProxyUrl || "",
  };
}

export function subscribeMediaProxyEndpoint(config: AppConfig, onChange: (value: string) => void) {
  return mediaProxyEndpoint.subscribe(config.mediaProxyUrl || "", onChange);
}

export function subscribePreviewEndpoints(
  config: AppConfig,
  onChange: (value: PreviewEndpoints) => void,
): () => void {
  let summaly: string | undefined;
  let media: string | undefined;
  function notify() {
    if (summaly !== undefined && media !== undefined) onChange({ summaly, media });
  }
  const unsubscribeSummaly = summalyEndpoint.subscribe(config.summalyUrl || "", (value) => {
    summaly = value;
    notify();
  });
  const unsubscribeMedia = subscribeMediaProxyEndpoint(config, (value) => {
    media = value;
    notify();
  });
  return () => {
    unsubscribeSummaly();
    unsubscribeMedia();
  };
}
