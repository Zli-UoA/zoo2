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
}: {
  avatar?: RemoteAvatar | null;
  name: string;
}) {
  const { config } = useCollections();
  return (
    <Avatar role="img" aria-label={name}>
      {avatar && /^[a-f0-9]{64}$/.test(avatar.image_id) ? (
        <AvatarImage
          src={joinUrl(config.apiBaseUrl, `/api/activitypub/emoji-images/${avatar.image_id}`)}
          alt=""
          loading="lazy"
          referrerPolicy="no-referrer"
        />
      ) : null}
      <AvatarFallback aria-hidden="true">{initials(name)}</AvatarFallback>
    </Avatar>
  );
}
