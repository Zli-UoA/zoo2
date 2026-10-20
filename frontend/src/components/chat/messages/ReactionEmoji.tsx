import { useState } from "react";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenImageUrl, kitchenLabel } from "../../../lib/emoji/kitchen";

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
  if (!url || failedUrl === url) return <span>{label}</span>;
  return (
    <img
      src={url}
      alt={label}
      width={size}
      height={size}
      className="inline-block shrink-0 object-contain"
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setFailedUrl(url)}
    />
  );
}
