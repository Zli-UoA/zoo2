import { useEffect, useMemo, useState } from "react";

import { useCollections } from "../../../lib/electric/collections-context";
import { createConfiguredApiClient } from "../../../lib/api/client";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenImageUrl, kitchenLabel, kitchenParts } from "../../../lib/emoji/kitchen";

function componentGlyph(component: string): string {
  return component
    .split("-")
    .map((hex) => String.fromCodePoint(Number.parseInt(hex, 16)))
    .join("");
}

export function ReactionEmoji({
  emoji,
  catalog,
  size = 28,
}: {
  emoji: string;
  catalog: KitchenCatalog | null;
  size?: number;
}) {
  const url = catalog ? kitchenImageUrl(catalog, emoji) : null;
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const label = kitchenLabel(emoji);
  const kitchen = kitchenParts(emoji);
  if (/^:[a-z0-9_-]{1,64}:$/.test(emoji))
    return <CustomReactionEmoji name={emoji.slice(1, -1)} size={size} />;
  if (!kitchen) return <span>{label}</span>;

  return (
    <span
      aria-hidden="true"
      className="inline-flex shrink-0 items-center justify-center"
      style={{ width: size, height: size }}
    >
      {url && failedUrl !== url ? (
        <img
          src={url}
          alt=""
          width={size}
          height={size}
          className="inline-block size-full object-contain"
          loading="lazy"
          referrerPolicy="no-referrer"
          onError={() => setFailedUrl(url)}
        />
      ) : (
        <span className="text-base leading-none">{componentGlyph(kitchen[0])}</span>
      )}
    </span>
  );
}

function CustomReactionEmoji({ name, size }: { name: string; size: number }) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    void client
      .get<{ url: string }>(`/api/emojis/${encodeURIComponent(name)}`, {
        signal: controller.signal,
      })
      .then((emoji) => {
        if (emoji.url.startsWith("https://")) setUrl(emoji.url);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [client, name]);
  return url ? (
    <img
      src={url}
      alt={`:${name}:`}
      width={size}
      height={size}
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setUrl(null)}
    />
  ) : (
    <span>{`:${name}:`}</span>
  );
}
