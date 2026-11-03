import { MfmContent } from "../chat/messages/MfmContent";
import { useState } from "react";
import { useCollections } from "../../lib/electric/collections-context";
import { joinUrl } from "../../lib/config";
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
}: {
  emoji: CustomEmoji;
  className?: string;
}) {
  const { config } = useCollections();
  const [failed, setFailed] = useState<string | null>(null);
  const ratio = emoji.width && emoji.height ? emoji.width / emoji.height : 1;
  const style = { width: "auto", maxWidth: "8em", aspectRatio: String(ratio) };
  if (emoji.local && (/^https?:\/\//.test(emoji.url) || /^\/(?!\/)/.test(emoji.url)))
    return (
      <img
        src={emoji.url.startsWith("/") ? joinUrl(config.apiBaseUrl, emoji.url) : emoji.url}
        alt={emoji.name}
        title={emoji.name}
        className={className}
        style={style}
        loading="lazy"
      />
    );
  if (!/^[a-f0-9]{64}$/.test(emoji.image_id)) return <span>{emoji.name}</span>;
  return (
    <img
      src={
        failed === emoji.image_id
          ? undefined
          : joinUrl(config.apiBaseUrl, `/api/activitypub/emoji-images/${emoji.image_id}`)
      }
      alt={emoji.name}
      title={emoji.name}
      className={className}
      style={style}
      loading="lazy"
      decoding="async"
      referrerPolicy="no-referrer"
      onError={() => setFailed(emoji.image_id)}
    />
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
        return emoji ? <ActivityPubEmoji emoji={emoji} className="mfm-custom-emoji" /> : undefined;
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
