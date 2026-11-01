export type StoryCallbacks = Record<string, (...args: unknown[]) => unknown>;

function revive(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(revive);
  if (!value || typeof value !== "object") return value;
  const record = value as Record<string, unknown>;
  if (record.__zooUndefined === true) return undefined;
  if (
    record.__zooFile === true &&
    typeof record.name === "string" &&
    typeof record.base64 === "string"
  ) {
    const bytes = Uint8Array.from(atob(record.base64), (char) => char.charCodeAt(0));
    return new File([bytes], record.name, {
      type: typeof record.type === "string" ? record.type : "",
    });
  }
  return Object.fromEntries(Object.entries(record).map(([key, item]) => [key, revive(item)]));
}

/** Execute the actual CSF callback, including rejection and File/undefined arguments. */
export async function dispatchStoryCallback(
  callbacks: StoryCallbacks,
  request: Record<string, unknown>,
  target: Window,
) {
  const reply = (value: Record<string, unknown>) =>
    target.postMessage(
      {
        type: "zoo-compose-catalog-callback-result",
        session: request.session,
        requestId: request.requestId,
        ...value,
      },
      window.location.origin,
    );
  try {
    const payload = JSON.parse(String(request.payload)) as { name: string; args?: unknown[] };
    if (!Object.hasOwn(callbacks, payload.name))
      throw new Error(`Unknown Story callback: ${payload.name}`);
    const result = await callbacks[payload.name](...(revive(payload.args ?? []) as unknown[]));
    reply({ ok: true, result: JSON.stringify(result ?? null) });
  } catch (error) {
    reply({ ok: false, error: error instanceof Error ? error.message : String(error) });
  }
}

export function collectStoryCallbacks(args: Record<string, unknown>): StoryCallbacks {
  const result: StoryCallbacks = {};
  const seen = new WeakSet<object>();
  function visit(value: unknown, path: string, depth: number) {
    if (typeof value === "function") {
      result[path] = (...items: unknown[]) => Reflect.apply(value, undefined, items);
      return;
    }
    if (!value || typeof value !== "object" || depth > 8 || seen.has(value)) return;
    seen.add(value);
    for (const [key, item] of Object.entries(value)) {
      if (["type", "_owner", "_store", "ref"].includes(key)) continue;
      visit(item, path ? `${path}.${key}` : key, depth + 1);
    }
  }
  visit(args, "", 0);
  return result;
}
