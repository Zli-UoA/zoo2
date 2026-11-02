import type { AppConfig } from "../../../lib/config";

export interface PreviewEndpoints {
  summaly: string;
  media: string;
}
let runtimeEndpoints: Promise<PreviewEndpoints> | undefined;
let runtimeValue: PreviewEndpoints | undefined;

async function readEndpoint(path: string): Promise<string> {
  const response = await fetch(path, { cache: "no-store", signal: AbortSignal.timeout(5_000) });
  if (!response.ok) throw new Error("Preview endpoint configuration unavailable");
  // The Vite development server serves HTML here; use its build-time configuration.
  return response.headers.get("content-type")?.startsWith("text/plain")
    ? (await response.text()).trim()
    : "";
}

function readRuntimeEndpoints(): Promise<PreviewEndpoints> {
  if (!runtimeEndpoints) {
    const request = Promise.all([
      readEndpoint("/summaly-endpoint"),
      readEndpoint("/media-proxy-endpoint"),
    ])
      .then(([summaly, media]) => {
        runtimeValue = { summaly, media };
        return runtimeValue;
      })
      .catch((error) => {
        if (runtimeEndpoints === request) runtimeEndpoints = undefined;
        throw error;
      });
    runtimeEndpoints = request;
  }
  return runtimeEndpoints;
}

function withFallback(value: PreviewEndpoints | null, config: AppConfig): PreviewEndpoints {
  return {
    summaly: value?.summaly || config.summalyUrl || "",
    media: value?.media || config.mediaProxyUrl || "",
  };
}

export function previewEndpoints(config: AppConfig): Promise<PreviewEndpoints> {
  return readRuntimeEndpoints().then((value) => withFallback(value, config));
}

const subscribers = new Set<(value: PreviewEndpoints | null) => void>();
let pending = false;
let timer: ReturnType<typeof setTimeout> | undefined;
let backoff = 2_000;

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
    const value = await readRuntimeEndpoints();
    for (const notify of subscribers) notify(value);
    stopRetrying();
  } catch {
    for (const notify of subscribers) notify(null);
    if (subscribers.size) {
      timer = setTimeout(refresh, backoff);
      backoff = Math.min(backoff * 2, 30_000);
    }
  } finally {
    pending = false;
  }
}

/** One retry schedule for the tab, regardless of the number of mounted cards. */
export function subscribePreviewEndpoints(
  config: AppConfig,
  onChange: (value: PreviewEndpoints) => void,
): () => void {
  if (runtimeValue) {
    onChange(withFallback(runtimeValue, config));
    return () => {};
  }
  const notify = (value: PreviewEndpoints | null) => onChange(withFallback(value, config));
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
