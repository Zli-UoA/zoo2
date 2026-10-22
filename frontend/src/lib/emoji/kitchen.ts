import catalogUrl from "./emoji-kitchen.json?url";

export interface KitchenCatalog {
  /** Source codepoint sequence, printable emoji, English search label. */
  emojis: [string, string, string][];
  dates: string[];
  /** Sorted emoji indices -> date index * 2 + reversed URL orientation. */
  pairs: Record<string, number>;
}

function componentKey(value: string): string | null {
  const hex = /^[\da-f]+(?:-[\da-f]+)*$/i.test(value)
    ? value.toLowerCase().split("-")
    : Array.from(value, (character) => character.codePointAt(0)!.toString(16));
  if (hex.length === 0 || hex.some((part) => Number.parseInt(part, 16) > 0x10ffff)) return null;
  return hex
    .filter((part) => part !== "fe0f")
    .map((part) => Number.parseInt(part, 16).toString(16))
    .join("-");
}

export function kitchenParts(value: string): [string, string] | null {
  if (!value.startsWith("kitchen:") || value.length > 256) return null;
  const parts = value.slice(8).split("_");
  if (parts.length !== 2 || parts.some((part) => !part)) return null;
  const left = componentKey(parts[0]);
  const right = componentKey(parts[1]);
  return left && right ? [left, right] : null;
}

/** Group Unicode/hex, variation-selector and reversed spellings together. */
export function reactionKey(value: string): string {
  const parts = kitchenParts(value);
  return parts ? `kitchen:${parts.sort().join("_")}` : value;
}

export function kitchenValue(left: string, right: string): string {
  return `kitchen:${[left, right].sort().join("_")}`;
}

export function kitchenLabel(value: string): string {
  const parts = kitchenParts(value);
  if (!parts) return value;
  return parts
    .map((part) => String.fromCodePoint(...part.split("-").map((hex) => Number.parseInt(hex, 16))))
    .join(" + ");
}

export interface KitchenEmoji {
  codepoint: string;
  emoji: string;
  name: string;
}

const catalogEmojis = new WeakMap<KitchenCatalog, readonly KitchenEmoji[]>();

export function kitchenEmojis(catalog: KitchenCatalog): readonly KitchenEmoji[] {
  let emojis = catalogEmojis.get(catalog);
  if (!emojis) {
    emojis = catalog.emojis.map(([codepoint, emoji, name]) => ({ codepoint, emoji, name }));
    catalogEmojis.set(catalog, emojis);
  }
  return emojis;
}

function normalizeQuery(value: string): string {
  return value.normalize("NFKC").toLowerCase().replaceAll("_", " ").replace(/\s+/g, " ").trim();
}

export function matchesKitchenQuery(emoji: KitchenEmoji, query: string): boolean {
  const tokens = normalizeQuery(query).split(" ").filter(Boolean);
  if (tokens.length === 0) return true;
  const text = `${normalizeQuery(emoji.name)} ${normalizeQuery(emoji.emoji)} ${emoji.codepoint}`;
  return tokens.every((token) => text.includes(token));
}

export function filterKitchenEmojis(
  emojis: readonly KitchenEmoji[],
  query: string,
): readonly KitchenEmoji[] {
  if (normalizeQuery(query) === "") return emojis;
  return emojis.filter((emoji) => matchesKitchenQuery(emoji, query));
}

const catalogIndices = new WeakMap<KitchenCatalog, ReadonlyMap<string, number>>();

function indexCatalog(catalog: KitchenCatalog): ReadonlyMap<string, number> {
  let indices = catalogIndices.get(catalog);
  if (!indices) {
    indices = new Map(
      catalog.emojis.map(([codepoint], index) => [componentKey(codepoint)!, index]),
    );
    catalogIndices.set(catalog, indices);
  }
  return indices;
}

export function kitchenImageUrl(catalog: KitchenCatalog, value: string): string | null {
  const parts = kitchenParts(value);
  if (!parts) return null;
  const lookup = indexCatalog(catalog);
  const indices = parts.map((part) => lookup.get(part) ?? -1);
  if (indices.some((index) => index < 0)) return null;
  const [first, second] = indices.sort((a, b) => a - b);
  const packed = catalog.pairs[`${first}_${second}`];
  if (packed === undefined) return null;
  const [left, right] = (packed % 2 ? [second, first] : [first, second]).map((index) =>
    catalog.emojis[index][0]
      .split("-")
      .map((part) => `u${part}`)
      .join("-"),
  );
  const date = catalog.dates[Math.floor(packed / 2)];
  return `https://www.gstatic.com/android/keyboard/emojikitchen/${date}/${left}/${left}_${right}.png`;
}

let catalogPromise: Promise<KitchenCatalog> | null = null;

/** Fetch the compact local catalog only when a Kitchen image or picker is used. */
export function loadKitchenCatalog(): Promise<KitchenCatalog> {
  catalogPromise ??= fetch(catalogUrl)
    .then(async (response) => {
      if (!response.ok) throw new Error("Emoji Kitchenの一覧を読み込めませんでした");
      const catalog = (await response.json()) as KitchenCatalog;
      indexCatalog(catalog);
      return catalog;
    })
    .catch((error: unknown) => {
      catalogPromise = null;
      throw error;
    });
  return catalogPromise;
}
