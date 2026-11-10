import { ImagePreview } from "../ui/image-preview";
import { imageAspectRatio, type ImageMetadata } from "../../lib/imageMetadata";
import { MessageAttachments } from "../chat/messages/MessageAttachments";
import type { AttachmentMetadata } from "../../lib/attachments";
import { useOptionalCollections } from "../../lib/electric/collections-context";
import { joinUrl, resolvePublicConfig } from "../../lib/config";
import { ExternalLink } from "../ui/external-link";
import { useTranslation } from "react-i18next";
import { ActivityPubContent, type CustomEmoji } from "./ActivityPubEmoji";

export interface HomeMedia extends ImageMetadata {
  url: string;
  name: string;
  media_type: string;
  image_id?: string;
}

/** The same MFM, emoji and link rendering in Home and public profiles. */
export function HomePostContent({
  content,
  emojis,
  media = [],
  attachments = [],
}: {
  content: string;
  emojis?: CustomEmoji[];
  media?: HomeMedia[];
  attachments?: AttachmentMetadata[];
}) {
  const { t } = useTranslation();
  const apiBaseUrl =
    useOptionalCollections()?.config.apiBaseUrl ?? resolvePublicConfig().apiBaseUrl;
  return (
    <div className="mfm-body mt-2 whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
      {content.trim() ? <ActivityPubContent text={content} emojis={emojis} /> : null}
      <MessageAttachments attachments={attachments} />
      {media.length && !attachments.length ? (
        <div className="mt-2 grid grid-cols-2 gap-2">
          {media.map((image, index) => {
            const path =
              image.image_id && /^[a-f0-9]{64}$/.test(image.image_id)
                ? `/api/activitypub/media-images/${image.image_id}`
                : /^\/api\/home\/images\/[a-f0-9-]+$/.test(image.url)
                  ? image.url
                  : null;
            if (!path) return null;
            const url = joinUrl(apiBaseUrl, path);
            return (
              <ExternalLink key={index} href={url} className="block min-w-0">
                <ImagePreview
                  {...image}
                  src={url}
                  alt={image.name || t("home.image")}
                  className="max-h-80 w-full"
                  style={{ aspectRatio: imageAspectRatio(image) }}
                />
              </ExternalLink>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
