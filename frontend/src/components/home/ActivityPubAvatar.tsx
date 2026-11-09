import { useCollections } from "../../lib/electric/collections-context";
import { joinUrl } from "../../lib/config";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { initials } from "../chat/format";

export interface RemoteAvatar {
  image_id: string;
}
export function ActivityPubAvatar({
  avatar,
  name,
  avatarUrl,
}: {
  avatar?: RemoteAvatar | null;
  name: string;
  avatarUrl?: string | null;
}) {
  const { config } = useCollections();
  const src = avatarUrl?.startsWith("https://")
    ? avatarUrl
    : avatar && /^[a-f0-9]{64}$/.test(avatar.image_id)
      ? joinUrl(config.apiBaseUrl, `/api/activitypub/emoji-images/${avatar.image_id}`)
      : undefined;
  return (
    <Avatar role="img" aria-label={name}>
      {src ? <AvatarImage src={src} alt="" loading="lazy" referrerPolicy="no-referrer" /> : null}
      <AvatarFallback aria-hidden="true">{initials(name)}</AvatarFallback>
    </Avatar>
  );
}
