import { eq, useLiveQuery } from "@tanstack/react-db";
import { AppAvatar } from "../chat/AppIdentity";
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
  userId,
}: {
  avatar?: RemoteAvatar | null;
  name: string;
  userId?: string;
}) {
  const { config, users } = useCollections();
  const localId = userId?.replace(/^local:/, "");
  const user = useLiveQuery(
    (q) => (localId ? q.from({ users }).where(({ users }) => eq(users.id, localId)) : undefined),
    [users, localId],
  )?.data?.[0];
  if (userId) {
    return (
      <span role="img" aria-label={name}>
        <AppAvatar label={name} src={user?.avatar_url} />
      </span>
    );
  }
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
