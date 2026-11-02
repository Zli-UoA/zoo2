import { useState } from "react";

import { useWorkspaceEmojis } from "../../../lib/emoji/workspace";
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
  size = 20,
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
  if (!kitchen)
    return (
      <span
        className="inline-flex shrink-0 items-center justify-center leading-none"
        style={{ width: size, height: size, fontSize: size }}
      >
        {label}
      </span>
    );

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
        <span className="leading-none" style={{ fontSize: size }}>
          {componentGlyph(kitchen[0])}
        </span>
      )}
    </span>
  );
}

function CustomReactionEmoji({ name, size }: { name: string; size: number }) {
  const { emojis, imageUrl } = useWorkspaceEmojis();
  const emoji = emojis.find((value) => value.name === name || value.aliases?.includes(name));
  const url = emoji ? imageUrl(emoji.url) : null;
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  return url && failedUrl !== url ? (
    <img
      src={url}
      alt={`:${name}:`}
      width={size}
      height={size}
      className="shrink-0 object-contain"
      style={{ width: "auto", maxWidth: size * 8, height: size }}
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setFailedUrl(url)}
    />
  ) : (
    <span>{`:${name}:`}</span>
  );
}
