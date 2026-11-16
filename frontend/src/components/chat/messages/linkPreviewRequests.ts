import { ApiError } from "../../../lib/api/client";
import type { SummalyClient, LinkPreviewData as Preview } from "../../../lib/summaly";

interface Entry {
  at: number;
  value: Preview | null;
  request: Promise<Preview | null>;
  controller: AbortController;
  users: number;
  settled: boolean;
}
const cache = new Map<string, Entry>();
const subscribers = new Map<string, Set<(data: Preview) => void>>();

/** Read without starting work, including while hidden/offline or before effects run. */
export function cachedPreview(key: string): Preview | null {
  return cache.get(key)?.value ?? null;
}

export function subscribePreview(key: string, notify: (data: Preview) => void) {
  let listeners = subscribers.get(key);
  if (!listeners) subscribers.set(key, (listeners = new Set()));
  listeners.add(notify);
  return () => {
    listeners.delete(notify);
    if (!listeners.size) subscribers.delete(key);
  };
}
let active = 0;
const waiting: (() => void)[] = [];

function trimCache() {
  for (const [key, entry] of cache) {
    if (cache.size <= 256) break;
    if (entry.settled) cache.delete(key);
  }
}

function slot(signal: AbortSignal): Promise<void> {
  signal.throwIfAborted();
  if (active < 4) {
    active++;
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const start = () => {
      signal.removeEventListener("abort", cancel);
      resolve();
    };
    const cancel = () => {
      waiting.splice(waiting.indexOf(start), 1);
      reject(signal.reason);
    };
    waiting.push(start);
    signal.addEventListener("abort", cancel, { once: true });
  });
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const cancel = () => {
      clearTimeout(timer);
      reject(signal.reason);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", cancel);
      resolve();
    }, ms);
    signal.addEventListener("abort", cancel, { once: true });
  });
}

async function retry(client: SummalyClient, url: string, signal: AbortSignal) {
  for (let attempt = 0; ; attempt++) {
    try {
      await slot(signal);
      try {
        signal.throwIfAborted();
        return await client.get(url, {
          signal: AbortSignal.any([signal, AbortSignal.timeout(12_000)]),
        });
      } finally {
        const next = waiting.shift();
        if (next) next();
        else active--;
      }
    } catch (error) {
      signal.throwIfAborted();
      // mei23/summaly reports all scrape errors as 422, including temporary origin errors.
      const transient =
        (error instanceof ApiError && [408, 422, 429, 502, 503, 504].includes(error.status)) ||
        error instanceof TypeError ||
        (error instanceof DOMException &&
          (error.name === "TimeoutError" || error.name === "AbortError"));
      if (!transient) return null;
      if (attempt >= 3) throw error;
      await delay(2_000 * 2 ** attempt, signal);
    }
  }
}

/** Serve the last success immediately; share a background revalidation after five minutes. */
export function acquirePreview(client: SummalyClient, key: string, url: string) {
  let entry = cache.get(key);
  if (
    !entry ||
    entry.controller.signal.aborted ||
    (entry.settled && Date.now() - entry.at >= 300_000)
  ) {
    const current: Entry = {
      at: entry?.at ?? Date.now(),
      value: entry?.value ?? null,
      controller: new AbortController(),
      users: 0,
      settled: false,
      request: Promise.resolve(null),
    };
    current.request = retry(client, url, current.controller.signal)
      .then((data) => {
        if (current.controller.signal.aborted || cache.get(key) !== current) return current.value;
        if (data) {
          current.value = data;
          for (const notify of subscribers.get(key) ?? []) notify(data);
        }
        return current.value;
      })
      .catch(() => {
        if (!current.value && cache.get(key) === current) cache.delete(key);
        return current.value;
      })
      .finally(() => {
        current.settled = true;
        // A failed refresh also waits five minutes before retrying; never erase
        // the previous value. Cancellation must allow the next view to resume.
        if (!current.controller.signal.aborted) current.at = Date.now();
        trimCache();
      });
    cache.set(key, current);
    // Never evict in-flight work: its mounted consumers still own cancellation.
    trimCache();
    entry = current;
  }
  // Touch settled entries without replacing the value used by mounted cards.
  cache.delete(key);
  cache.set(key, entry);
  entry.users++;
  const owned = entry;
  let released = false;
  return {
    request: owned.request,
    release() {
      if (released) return;
      released = true;
      owned.users--;
      if (!owned.users && !owned.settled) {
        if (!owned.value && cache.get(key) === owned) cache.delete(key);
        owned.controller.abort();
      }
    },
  };
}
