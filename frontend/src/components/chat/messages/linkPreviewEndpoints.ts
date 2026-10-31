import type { AppConfig } from "../../../lib/config";

export interface PreviewEndpoints {
  summaly: string;
  media: string;
}
let runtimeEndpoints: Promise<PreviewEndpoints> | undefined;

async function readEndpoint(path: string): Promise<string> {
  const response = await fetch(path, { cache: "no-store", signal: AbortSignal.timeout(5_000) });
  if (!response.ok) throw new Error("Preview endpoint configuration unavailable");
  // The Vite development server serves HTML here; use its build-time configuration.
  return response.headers.get("content-type")?.startsWith("text/plain")
    ? (await response.text()).trim()
    : "";
}

export function previewEndpoints(config: AppConfig): Promise<PreviewEndpoints> {
  if (!runtimeEndpoints) {
    const request = Promise.all([
      readEndpoint("/summaly-endpoint"),
      readEndpoint("/media-proxy-endpoint"),
    ])
      .then(([summaly, media]) => ({ summaly, media }))
      .catch((error) => {
        if (runtimeEndpoints === request) runtimeEndpoints = undefined;
        throw error;
      });
    runtimeEndpoints = request;
  }
  return runtimeEndpoints.then((value) => ({
    summaly: value.summaly || config.summalyUrl || "",
    media: value.media || config.mediaProxyUrl || "",
  }));
}
