import { ActivityPubContent, type CustomEmoji } from "./ActivityPubEmoji";
import { MessageAttachments } from "../chat/messages/MessageAttachments";
import type { AttachmentMetadata } from "../../lib/attachments";

/** Home reuses channel text and attachment rendering, including image previews. */
export function HomePostContent({
  content,
  emojis,
  attachments = [],
}: {
  content: string;
  emojis?: CustomEmoji[];
  attachments?: AttachmentMetadata[];
}) {
  return (
    <>
      {content.trim() ? (
        <div className="mfm-body mt-2 whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
          <ActivityPubContent text={content} emojis={emojis} />
        </div>
      ) : null}
      <MessageAttachments attachments={attachments} />
    </>
  );
}
