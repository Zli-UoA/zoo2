import { useState } from "react";

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
