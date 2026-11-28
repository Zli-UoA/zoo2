import { Link } from "@tanstack/react-router";
import { memo, type ReactNode } from "react";
import { SensitiveMedia } from "../ui/sensitive-media";
import { MediaPreview } from "../ui/media-preview";
import { type ImageMetadata } from "../../lib/imageMetadata";
import { LinkPreviews } from "../chat/messages/LinkPreviews.tsx";
import { MessageAttachments } from "../chat/messages/MessageAttachments";
import type { AttachmentMetadata } from "../../lib/attachments";
import { useOptionalCollections } from "../../lib/electric/collections-context";
import { joinUrl, resolvePublicConfig } from "../../lib/config";
import { ExternalLink, inlineLinkClassName } from "../ui/external-link";
import { useTranslation } from "react-i18next";
import { ActivityPubAvatar, type RemoteAvatar } from "./ActivityPubAvatar";
import { ActivityPubContent, ActivityPubText, type CustomEmoji } from "./ActivityPubEmoji";

export interface HomeMedia extends ImageMetadata {
  url: string;
  name: string;
  media_type: string;
  description?: string;
  image_id?: string;
  is_sensitive?: boolean;
}

export interface HomeQuote {
  uri: string;
  local?: boolean;
  post: {
    id: string;
    remote?: boolean;
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

function QuotePostLink({
  quote,
  className,
  children,
}: {
  quote: HomeQuote;
  className: string;
  children: ReactNode;
}) {
  if (quote.post?.remote === false) {
    return (
      <Link
        to="/home/posts/$postId"
        params={{ postId: quote.post.id }}
        className={`${inlineLinkClassName} ${className}`}
      >
        {children}
      </Link>
    );
  }
  return (
    <ExternalLink href={quote.uri} className={className}>
      {children}
    </ExternalLink>
  );
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
          <QuotePostLink quote={quote} className="min-w-0">
            <span className="flex min-w-0 items-center gap-2">
              <ActivityPubAvatar name={name} avatar={post.avatar} avatarUrl={post.avatar_url} />
              <span className="min-w-0">
                <span className="block break-words font-semibold text-foreground">
                  <ActivityPubText text={name} emojis={post.actor_emojis} />
                </span>
                <span className="block break-words text-xs text-muted-foreground">
                  @{post.user_name}
                </span>
              </span>
            </span>
          </QuotePostLink>
          <HomePostContent
            content={post.content}
            emojis={post.emojis}
            media={post.media}
            previewLinks={false}
          />
          <QuotePostLink quote={quote} className="mt-2 inline-block text-xs text-link">
            <time dateTime={post.created_at}>
              {new Date(post.created_at).toLocaleString(i18n.language)}
            </time>
          </QuotePostLink>
        </>
      ) : (
        <>
          <p className="text-muted-foreground">{t("home.quoteUnavailable")}</p>
          {!quote.local ? (
            <ExternalLink href={quote.uri} className="mt-1 block break-words text-link">
              {t("home.openQuotedPost")}
            </ExternalLink>
          ) : null}
        </>
      )}
    </div>
  );
}

/** The same MFM, emoji and link rendering in Home and public profiles. */
export const HomePostContent = memo(function HomePostContent({
  content,
  emojis,
  media = [],
  attachments = [],
  quote,
  previewLinks = true,
  excludePreviewUrl,
}: {
  content: string;
  emojis?: CustomEmoji[];
  media?: HomeMedia[];
  attachments?: AttachmentMetadata[];
  quote?: HomeQuote | null;
  previewLinks?: boolean;
  excludePreviewUrl?: string;
}) {
  const { t } = useTranslation();
  const apiBaseUrl =
    useOptionalCollections()?.config.apiBaseUrl ?? resolvePublicConfig().apiBaseUrl;
  // Remove only the standalone link appended by the receiver, not links in the comment.
  const body =
    quote && content.endsWith(`\n\nRE: <${quote.uri}>`)
      ? content.slice(0, -`\n\nRE: <${quote.uri}>`.length)
      : quote && content.endsWith(`<${quote.uri}>`)
        ? content === `<${quote.uri}>`
          ? ""
          : content.endsWith(`\n\n<${quote.uri}>`)
            ? content.slice(0, -`\n\n<${quote.uri}>`.length)
            : content
        : content;
  return (
    <div className="mfm-body mt-2 whitespace-pre-wrap break-words wrap-anywhere text-sm leading-relaxed">
      {body.trim() ? <ActivityPubContent text={body} emojis={emojis} /> : null}
      {previewLinks ? (
        <LinkPreviews
          content={body}
          excludeUrl={quote?.uri ?? excludePreviewUrl}
          defer
          showFallback={false}
        />
      ) : null}
      {quote ? <QuoteCard quote={quote} /> : null}
      <MessageAttachments attachments={attachments} />
      {media.length && !attachments.length ? (
        <div className="mt-2 grid grid-cols-1 gap-2">
          {media.map((image, index) => {
            const path =
              image.image_id && /^[a-f0-9]{64}$/.test(image.image_id)
                ? `/api/activitypub/media-images/${image.image_id}`
                : /^\/api\/home\/images\/[a-f0-9-]+$/.test(image.url)
                  ? image.url
                  : null;
            // Remote images retain the registered proxy. Other passive formats use HTTPS.
            if (!path && (!/^https:\/\//i.test(image.url) || image.media_type.startsWith("image/")))
              return null;
            const url = path ? joinUrl(apiBaseUrl, path) : image.url;
            return (
              <SensitiveMedia
                key={`${url}:${Boolean(image.is_sensitive)}:${index}`}
                sensitive={image.is_sensitive}
                preview={image}
              >
                <MediaPreview
                  {...image}
                  src={url}
                  thumbnailSrc={
                    path?.startsWith("/api/home/images/") ? `${url}?thumbnail=true` : undefined
                  }
                  alt={image.description}
                  name={image.name || t("home.image")}
                  contentType={image.media_type}
                  saveId={image.image_id ? `home:${image.image_id}` : `home:${url}`}
                />
              </SensitiveMedia>
            );
          })}
        </div>
      ) : null}
    </div>
  );
});
