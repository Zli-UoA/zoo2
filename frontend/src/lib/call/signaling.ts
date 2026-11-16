import { buildAuthHeaders } from "../api/client";
import { joinUrl, type AppConfig } from "../config";

/** One authenticated SSE transport for DM/channel and gateway calls. */
export async function openCallStream(
  config: AppConfig,
  path: string,
  options: { signal: AbortSignal; method?: "GET" | "POST"; onAuth?: (headers: Headers) => void },
) {
  const headers = await buildAuthHeaders({ devUserId: config.devUserId });
  options.signal.throwIfAborted();
  options.onAuth?.(new Headers(headers));
  options.signal.throwIfAborted();
  headers.set("Accept", "text/event-stream");
  return fetch(joinUrl(config.apiBaseUrl, path), {
    method: options.method ?? "GET",
    headers,
    signal: options.signal,
  });
}
export async function* readCallEvents(
  response: Response,
): AsyncGenerator<{ eventType: string; data: Record<string, unknown> }> {
  const reader = response.body?.getReader();
  if (!reader) throw new Error("Call stream body is empty");
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) return;
      buffer += decoder.decode(value, { stream: true });
      const parts = buffer.split(/\r?\n\r?\n/);
      buffer = parts.pop() ?? "";
      if (buffer.length > 65_536) throw new Error("Call event too large");
      for (const part of parts) {
        if (part.length > 65_536) throw new Error("Call event too large");
        let eventType = "message";
        let body = "";
        for (const line of part.split(/\r?\n/)) {
          if (line.startsWith("event:")) eventType = line.slice(6).trim();
          else if (line.startsWith("data:")) body += (body ? "\n" : "") + line.slice(5).trimStart();
        }
        if (!body) continue;
        let data: unknown;
        try {
          data = JSON.parse(body);
        } catch {
          continue;
        }
        if (data && typeof data === "object" && !Array.isArray(data))
          yield { eventType, data: data as Record<string, unknown> };
      }
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
}
