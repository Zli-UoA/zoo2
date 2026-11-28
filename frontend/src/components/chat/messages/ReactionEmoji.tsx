import { ActivityPubEmoji } from "../../home/ActivityPubEmoji";
import { useState, type CSSProperties } from "react";
import { AccountGlyph } from "./MfmAccounts";

import { useWorkspaceEmojis } from "../../../lib/emoji/workspace";
import type { DisplayEmoji } from "../../../lib/electric/types";
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
  emojis,
}: {
  emojis?: Record<string, DisplayEmoji>;
  emoji: string;
  catalog: KitchenCatalog | null;
  size?: number;
}) {
  const url = catalog ? kitchenImageUrl(catalog, emoji) : null;
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  if (/^:@[a-z0-9._-]{1,80}:$/.test(emoji))
    return (
      <span
        className="text-(length:--emoji-size)"
        style={{ "--emoji-size": `${size / 1.15}px` } as CSSProperties}
      >
        <AccountGlyph name={emoji.slice(2, -1)} />
      </span>
    );
  const label = kitchenLabel(emoji);
  const kitchen = kitchenParts(emoji);
  const remote = /^apub:([a-f0-9]{64}):(:[A-Za-z0-9_+-]+:)$/.exec(emoji);
  if (remote)
    return (
      <span
        className="inline-flex h-(--emoji-size) shrink-0 text-(length:--emoji-size)"
        style={{ "--emoji-size": `${size}px` } as CSSProperties}
      >
        <ActivityPubEmoji
          emoji={{ ...emojis?.[emoji], image_id: remote[1]!, name: remote[2]!, url: "" }}
          className="inline-block h-[1em] object-contain"
          size={size}
        />
      </span>
    );
  if (/^:[a-z0-9_-]{1,64}:$/.test(emoji))
    return <CustomReactionEmoji name={emoji.slice(1, -1)} size={size} emojis={emojis} />;
  if (!kitchen)
    return (
      <span
        className="inline-flex size-(--emoji-size) shrink-0 items-center justify-center text-(length:--emoji-size) leading-none"
        style={{ "--emoji-size": `${size}px` } as CSSProperties}
      >
        {label}
      </span>
    );

  return (
    <span
      aria-hidden="true"
      className="inline-flex size-(--emoji-size) shrink-0 items-center justify-center"
      style={{ "--emoji-size": `${size}px` } as CSSProperties}
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
        <span
          className="text-(length:--emoji-size) leading-none"
          style={{ "--emoji-size": `${size}px` } as CSSProperties}
        >
          {componentGlyph(kitchen[0])}
        </span>
      )}
    </span>
  );
}

function CustomReactionEmoji({
  name,
  size,
  emojis: joined,
}: {
  name: string;
  size: number;
  emojis?: Record<string, DisplayEmoji>;
}) {
  const { emojis, imageUrl } = useWorkspaceEmojis(joined === undefined);
  const emoji =
    joined !== undefined
      ? joined[name]
      : emojis.find((value) => value.name === name || value.aliases?.includes(name));
  const url = emoji ? imageUrl(emoji.url) : null;
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const ratio = emoji?.width && emoji.height ? emoji.width / emoji.height : 1;
  const emojiVars = {
    "--emoji-height": `${size}px`,
    "--emoji-width": `${size * ratio}px`,
    "--emoji-max-width": `${size * 8}px`,
  } as CSSProperties;
  return url && failedUrl !== url ? (
    <img
      src={url}
      alt={`:${name}:`}
      width={size}
      height={size}
      className="h-(--emoji-height) w-(--emoji-width) max-w-(--emoji-max-width) shrink-0 object-contain"
      style={emojiVars}
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setFailedUrl(url)}
    />
  ) : (
    <span
      className={
        emoji
          ? "inline-block h-(--emoji-height) w-(--emoji-width) max-w-(--emoji-max-width) shrink-0 overflow-hidden align-middle"
          : undefined
      }
      style={emoji ? emojiVars : undefined}
      title={`:${name}:`}
    >{`:${name}:`}</span>
  );
}
