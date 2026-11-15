import { SensitiveMedia } from "../ui/sensitive-media";
import { ImagePreview } from "../ui/image-preview";
import { imageAspectRatio, type ImageMetadata } from "../../lib/imageMetadata";
import { LinkPreviews } from "../chat/messages/LinkPreviews";
import { MessageAttachments } from "../chat/messages/MessageAttachments";
import type { AttachmentMetadata } from "../../lib/attachments";
import { useOptionalCollections } from "../../lib/electric/collections-context";
import { joinUrl, resolvePublicConfig } from "../../lib/config";
import { ExternalLink } from "../ui/external-link";
import { useTranslation } from "react-i18next";
import { ActivityPubAvatar, type RemoteAvatar } from "./ActivityPubAvatar";
import { ActivityPubContent, ActivityPubText, type CustomEmoji } from "./ActivityPubEmoji";

export interface HomeMedia extends ImageMetadata {
  url: string;
  name: string;
  media_type: string;
  image_id?: string;
  is_sensitive?: boolean;
}

export interface HomeQuote {
  uri: string;
  post: {
    id: string;
    content: string;
    user_name: string;
    display_name: string;
    created_at: string;
    emojis?: CustomEmoji[];
    actor_emojis?: CustomEmoji[];
    avatar?: RemoteAvatar | null;
    avatar_url?: string | null;
    media?: HomeMedia[];
  } | null;
}

function QuoteCard({ quote }: { quote: HomeQuote }) {
  const { t, i18n } = useTranslation();
  const post = quote.post;
  const name = post?.display_name || post?.user_name || "";
  return (
    <div
      role="group"
      aria-label={t("home.quotedPost")}
      className="mt-3 min-w-0 rounded-xl border bg-card p-3 text-sm"
    >
      {post ? (
        <>
          <ExternalLink href={quote.uri} className="flex min-w-0 items-center gap-2">
            <ActivityPubAvatar name={name} avatar={post.avatar} avatarUrl={post.avatar_url} />
            <span className="min-w-0">
              <span className="block break-words font-semibold text-foreground">
                <ActivityPubText text={name} emojis={post.actor_emojis} />
              </span>
              <span className="block break-words text-xs text-muted-foreground">
                @{post.user_name}
              </span>
            </span>
          </ExternalLink>
          <HomePostContent
            content={post.content}
            emojis={post.emojis}
            media={post.media}
            previewLinks={false}
          />
          <ExternalLink href={quote.uri} className="mt-2 inline-block text-xs text-link">
            <time dateTime={post.created_at}>
              {new Date(post.created_at).toLocaleString(i18n.language)}
            </time>
          </ExternalLink>
        </>
      ) : (
        <>
          <p className="text-muted-foreground">{t("home.quoteUnavailable")}</p>
          <ExternalLink href={quote.uri} className="mt-1 block break-words text-link">
            {t("home.openQuotedPost")}
          </ExternalLink>
        </>
      )}
    </div>
  );
}

/** The same MFM, emoji and link rendering in Home and public profiles. */
export function HomePostContent({
  content,
  emojis,
  media = [],
  attachments = [],
  quote,
  previewLinks = true,
}: {
  content: string;
  emojis?: CustomEmoji[];
  media?: HomeMedia[];
  attachments?: AttachmentMetadata[];
  quote?: HomeQuote | null;
  previewLinks?: boolean;
}) {
  const { t } = useTranslation();
  const apiBaseUrl =
    useOptionalCollections()?.config.apiBaseUrl ?? resolvePublicConfig().apiBaseUrl;
  // Remove only the standalone link appended by the receiver, not links in the comment.
  const body =
    quote && content.endsWith(`<${quote.uri}>`)
      ? content === `<${quote.uri}>`
        ? ""
        : content.endsWith(`\n\n<${quote.uri}>`)
          ? content.slice(0, -`\n\n<${quote.uri}>`.length)
          : content
      : content;
  return (
    <div className="mfm-body mt-2 whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
      {body.trim() ? <ActivityPubContent text={body} emojis={emojis} /> : null}
      {previewLinks ? (
        <LinkPreviews content={body} excludeUrl={quote?.uri} defer showFallback={false} />
      ) : null}
      {quote ? <QuoteCard quote={quote} /> : null}
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
              <SensitiveMedia
                key={`${url}:${Boolean(image.is_sensitive)}:${index}`}
                sensitive={image.is_sensitive}
                preview={image}
              >
                <ExternalLink href={url} className="block min-w-0">
                  <ImagePreview
                    {...image}
                    src={url}
                    alt={image.name || t("home.image")}
                    className="max-h-80 w-full"
                    style={{ aspectRatio: imageAspectRatio(image) }}
                  />
                </ExternalLink>
              </SensitiveMedia>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
