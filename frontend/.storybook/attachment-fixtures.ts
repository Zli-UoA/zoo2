import type { AttachmentRow } from "../src/lib/electric/types";

export const storyAttachments: AttachmentRow[] = [
  {
    id: "attachment-image-1",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-general",
    file_name: "screenshot.png",
    file_size: "184320",
    content_type: "image/png",
    storage_key: "attachments/channel-general/attachment-image-1",
    created_at: "2026-01-02 09:00:00.000",
  },
  {
    id: "attachment-image-2",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-general",
    file_name: "diagram.png",
    file_size: "98304",
    content_type: "image/png",
    storage_key: "attachments/channel-general/attachment-image-2",
    created_at: "2026-01-02 09:00:01.000",
  },
  {
    id: "attachment-file",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-general",
    file_name: "release-notes.pdf",
    file_size: "524288",
    content_type: "application/pdf",
    storage_key: "attachments/channel-general/attachment-file",
    created_at: "2026-01-02 09:00:02.000",
  },
];
