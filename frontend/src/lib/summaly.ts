import { ApiError } from "./api/client";

export interface LinkPreviewData {
  title: string;
  description: string;
  image: string | null;
}
export interface SummalyClient {
  get(url: string, options: { signal: AbortSignal }): Promise<LinkPreviewData>;
}

/** Summaly is public: app credentials and referrers must never be forwarded. */
export function createSummalyClient(endpoint: string): SummalyClient {
  return {
    async get(url, { signal }) {
      const target = new URL(endpoint, globalThis.location?.origin ?? "http://localhost");
      if (
        (target.protocol !== "https:" &&
          !(target.protocol === "http:" && /^(localhost|127\.0\.0\.1)$/.test(target.hostname))) ||
        target.username ||
        target.password
      )
        throw new ApiError(400, "Unsupported Summaly endpoint");
      target.searchParams.set("url", url);
      const response = await fetch(target.href, {
        signal,
        credentials: "omit",
        referrerPolicy: "no-referrer",
        headers: { Accept: "application/json" },
      });
      if (!response.ok) throw new ApiError(response.status, "Summaly request failed");
      const text = await response.text();
      if (text.length > 65_536) throw new ApiError(502, "Summaly response is too large");
      return parseSummaly(JSON.parse(text), url);
    },
  };
}

export function parseSummaly(value: unknown, url: string): LinkPreviewData {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new ApiError(502, "Invalid Summaly response");
  const data = value as Record<string, unknown>;
  return {
    title: (typeof data.title === "string" && data.title.trim()
      ? data.title.trim()
      : new URL(url).hostname
    ).slice(0, 300),
    description: typeof data.description === "string" ? data.description.slice(0, 1000) : "",
    image: typeof data.thumbnail === "string" ? data.thumbnail : null,
  };
}
