export { EVENT_TYPES, type EventType } from "./events.ts";
export interface Credentials { access_token: string; refresh_token: string; expires_in?: number }
export interface AppEvent { id: string; version: 1; type: string; actor_id: string | null; channel_id?: string | null; subject_user_id?: string | null; data: Record<string, unknown> }
export interface Delivery { delivery_id: string; ack_token: string; event: AppEvent; attempt: number }
export interface EventStore { has(id: string): Promise<boolean>; mark(id: string): Promise<void> }
export interface Options { baseUrl: string; credentials: Credentials; saveCredentials?: (credentials: Credentials) => Promise<void>; fetch?: typeof fetch; eventStore?: EventStore }
export class ZooError extends Error { readonly status: number; constructor(status: number, message: string) { super(message); this.status=status; } }
export class ZooApp {
  private credentials: Credentials;
  private refreshPromise: Promise<void> | undefined;
  private readonly fetcher: typeof fetch;
  private readonly base: string;
  private readonly seen = new Set<string>();
  private readonly inFlight = new Map<string, Promise<void>>();
  private readonly options: Options;
  constructor(options: Options) { this.options=options; this.base = options.baseUrl.replace(/\/$/, ""); this.credentials = options.credentials; this.fetcher = options.fetch ?? fetch; }
  private async refresh() {
    this.refreshPromise ??= (async () => {
      const response = await this.fetcher(`${this.base}/api/apps/oauth/token`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ grant_type: "refresh_token", refresh_token: this.credentials.refresh_token }) });
      if (!response.ok) throw new ZooError(response.status, "Could not refresh app credentials");
      this.credentials = await response.json() as Credentials;
      await this.options.saveCredentials?.(this.credentials);
    })().finally(() => { this.refreshPromise = undefined; });
    await this.refreshPromise;
  }
  async request<T = unknown>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
    if (!path.startsWith("/") || path.startsWith("//")) throw new Error("An API path is required");
    for (let attempt = 0; attempt < 3; attempt++) {
      const access = this.credentials.access_token;
      const response = await this.fetcher(`${this.base}/api/apps/v1${path}`, { method, signal, headers: { authorization: `Bearer ${access}`, "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
      if (response.status === 401 && attempt < 2) { if (this.credentials.access_token === access) await this.refresh(); continue; }
      if (response.status === 429 && attempt < 2) { await delay(Math.min(60, Number(response.headers.get("retry-after")) || 1) * 1000, signal); continue; }
      const value: unknown = await response.json();
      if (!response.ok) throw new ZooError(response.status, typeof value === "object" && value && "error" in value ? String(value.error) : "Zoo API request failed");
      if (typeof value === "object" && value && "txid" in value && "data" in value) return value.data as T;
      return value as T;
    }
    throw new Error("Zoo API retry limit reached");
  }
  postMessage(input: { channel_id: string; content: string; parent_id?: string; id?: string; kind?: "message" | "emote"; broadcast?: boolean; metadata?: Record<string, unknown> }) { return this.request<{ id: string }>("POST", "/messages", { ...input, id: input.id ?? crypto.randomUUID() }); }
  updateMessage(id: string, content: string) { return this.request("PATCH", `/messages/${encodeURIComponent(id)}`, { content }); }
  deleteMessage(id: string) { return this.request("DELETE", `/messages/${encodeURIComponent(id)}`); }
  setBlocks(id: string, blocks: unknown[]) { return this.request("POST", `/messages/${encodeURIComponent(id)}/blocks`, blocks); }
  publish(input: { kind: "home" | "modal" | "ephemeral"; user_id: string; channel_id?: string; trigger_id?: string; blocks: unknown[] }) { return this.request("POST", "/surfaces", input); }
  openDm(userId: string) { return this.request<{ id: string }>("POST", "/dms", { target_user_ids: [userId] }); }
  async dispatch(delivery: Delivery, handler: (event: AppEvent) => Promise<void>) {
    const id = delivery.event.id;
    if (this.seen.has(id)) return;
    let pending = this.inFlight.get(id);
    if (!pending) {
      // Reserve the ID before awaiting the durable store, including slow or failed reads.
      pending = Promise.resolve().then(async () => {
        if (this.seen.has(id) || await this.options.eventStore?.has(id)) return;
        await handler(delivery.event);
        await this.options.eventStore?.mark(id);
        this.seen.add(id);
        if (this.seen.size > 1000) this.seen.delete(this.seen.values().next().value!);
      }).finally(() => this.inFlight.delete(id));
      this.inFlight.set(id, pending);
    }
    await pending;
  }
  /** Receive with reconnects. ACK follows successful processing; use a durable EventStore across restarts. */
  async connect(handler: (event: AppEvent) => Promise<void>, signal?: AbortSignal) {
    let backoff = 500;
    while (!signal?.aborted) {
      try {
        const ticket = await this.request<{ ticket: string; path: string }>("POST", "/socket-ticket", undefined, signal);
        const url = new URL(ticket.path, this.base); url.protocol = url.protocol === "https:" ? "wss:" : "ws:"; url.searchParams.set("ticket", ticket.ticket);
        await new Promise<void>((resolve) => {
          const ws = new WebSocket(url);
          const abort = () => ws.close(); signal?.addEventListener("abort", abort, { once: true });
          ws.onopen = () => { backoff = 500; if (signal?.aborted) ws.close(); };
          ws.onmessage = (message) => {
            let delivery: Delivery;
            try { delivery = JSON.parse(String(message.data)) as Delivery; if (!delivery.event?.id || !delivery.ack_token) return; } catch { return; }
            void this.dispatch(delivery, handler).then(() => { if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ ack: delivery.delivery_id, ack_token: delivery.ack_token })); }).catch(() => { /* Do not ACK failed processing; Zoo retries it. */ });
          };
          ws.onerror = () => ws.close();
          ws.onclose = () => { signal?.removeEventListener("abort", abort); resolve(); };
        });
      } catch (error) { if (signal?.aborted) break; if (error instanceof ZooError && (error.status === 401 || error.status === 403)) throw error; }
      if (signal?.aborted) break;
      try { await delay(backoff, signal); } catch (error) { if (signal?.aborted) break; throw error; } backoff = Math.min(backoff * 2, 30_000);
    }
  }
  /** Electric protocol passthrough, suitable for an Electric client or one-shot shape reads. */
  async shape(name: "messages" | "channels" | "users" | "channel_users" | "message_reactions", params: Record<string, string>, signal?: AbortSignal) {
    const path = `/shapes/${name}?${new URLSearchParams(params)}`;
    for(let attempt=0;attempt<2;attempt++) {
      const access=this.credentials.access_token;
      const response = await this.fetcher(`${this.base}/api/apps/v1${path}`, { headers: { authorization: `Bearer ${access}` }, signal });
      if (response.status === 401 && attempt===0) { if(this.credentials.access_token===access) await this.refresh(); continue; }
      if (!response.ok) throw new ZooError(response.status, "Shape request failed");
      return response;
    }
    throw new Error("Shape retry limit reached");
  }
}
function delay(ms: number, signal?: AbortSignal): Promise<void> { return new Promise((resolve, reject) => { if (signal?.aborted) { reject(signal.reason); return; } const abort = () => { clearTimeout(timer); reject(signal?.reason); }; const timer = setTimeout(() => { signal?.removeEventListener("abort", abort); resolve(); }, ms); signal?.addEventListener("abort", abort, { once: true }); }); }

/** Verify the raw HTTP body before JSON parsing. Timestamps have a five-minute tolerance. */
export async function verifySignature(secret: string, timestamp: string, body: string, signature: string, now = Date.now()): Promise<boolean> {
  if (!/^\d+$/.test(timestamp) || Math.abs(now / 1000 - Number(timestamp)) > 300 || !/^v1=[a-f0-9]{64}$/.test(signature)) return false;
  const encoder = new TextEncoder();
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["verify"]);
  const bytes = Uint8Array.from(signature.slice(3).match(/../g)!, (b) => Number.parseInt(b, 16));
  return crypto.subtle.verify("HMAC", key, bytes, encoder.encode(`v1:${timestamp}:${body}`));
}
