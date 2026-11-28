import { MfmContent } from "../chat/messages/MfmContent";
import { useState, type CSSProperties } from "react";
import { useCollections } from "../../lib/electric/collections-context";
import { joinUrl } from "../../lib/config";
import { emojiImageStyle } from "../../lib/emoji/dimensions";
export interface CustomEmoji {
  width?: number | null;
  height?: number | null;
  name: string;
  url: string;
  id?: string | null;
  image_id: string;
  local?: boolean;
  category?: string;
  aliases?: string[];
}
export function ActivityPubEmoji({
  emoji,
  className = "inline-block size-[1.3em] align-middle",
  size = "1.3em",
}: {
  emoji: CustomEmoji;
  className?: string;
  size?: string | number;
}) {
  const { config } = useCollections();
  const [failed, setFailed] = useState<string | null>(null);
  const dimensions = emojiImageStyle(emoji, size);
  // カスタムプロパティは React が数値に px を付けないので、ここで文字列に直す。
  const px = (value: string | number | undefined) =>
    typeof value === "number" ? `${value}px` : value;
  const style = {
    "--emoji-height": px(dimensions.height),
    "--emoji-width": px(dimensions.width),
    "--emoji-max-width": px(dimensions.maxWidth),
  } as CSSProperties;
  const local = emoji.local && (/^https?:\/\//.test(emoji.url) || /^\/(?!\/)/.test(emoji.url));
  if (!local && !/^[a-f0-9]{64}$/.test(emoji.image_id)) return <span>{emoji.name}</span>;
  const src = local
    ? emoji.url.startsWith("/")
      ? joinUrl(config.apiBaseUrl, emoji.url)
      : emoji.url
    : joinUrl(config.apiBaseUrl, `/api/activitypub/emoji-images/${emoji.image_id}`);
  return (
    <span
      className={`inline-block ${className} h-(--emoji-height) w-(--emoji-width) max-w-(--emoji-max-width) overflow-hidden whitespace-nowrap`}
      style={style}
      title={emoji.name}
    >
      {failed === emoji.image_id ? (
        emoji.name
      ) : (
        <img
          src={src}
          alt={emoji.name}
          className="block h-full w-full object-contain"
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setFailed(emoji.image_id)}
        />
      )}
    </span>
  );
}
/** Same MFM parser and rendering as channels, retaining remote emoji proxy/fallbacks. */
export function ActivityPubContent({ text, emojis }: { text: string; emojis?: CustomEmoji[] }) {
  return (
    <MfmContent
      content={text}
      emojis={
        emojis === undefined
          ? undefined
          : Object.fromEntries(emojis.map((emoji) => [emoji.name.replace(/^:|:$/g, ""), emoji]))
      }
      renderEmoji={(name) => {
        const emoji = emojis?.find((value) => value.name === `:${name}:`);
        return emoji ? (
          <ActivityPubEmoji emoji={emoji} className="mfm-custom-emoji" size="1.5em" />
        ) : undefined;
      }}
    />
  );
}
export function ActivityPubText({ text, emojis = [] }: { text: string; emojis?: CustomEmoji[] }) {
  return (
    <>
      {text.split(/(:[A-Za-z0-9_+-]+:)/g).map((part, index) => {
        const emoji = emojis.find((e) => e.name === part);
        return emoji ? <ActivityPubEmoji key={`${index}:${emoji.image_id}`} emoji={emoji} /> : part;
      })}
    </>
  );
}
export function FederatedAccount({
  userName,
  actorId,
  remote,
}: {
  userName: string;
  actorId: string;
  remote?: boolean;
}) {
  let host = "";
  if (remote) {
    try {
      host = new URL(actorId).host;
    } catch {
      host = userName.includes("@") ? userName.slice(userName.lastIndexOf("@") + 1) : "";
    }
  }
  const name =
    host && userName.endsWith(`@${host}`) ? userName.slice(0, -host.length - 1) : userName;
  return (
    <span className="break-all">
      <span className="text-foreground">@{name}</span>
      {host ? <span className="text-muted-foreground">@{host}</span> : null}
    </span>
  );
}
