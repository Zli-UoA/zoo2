-- Create "attachments" table
CREATE TABLE attachments (
  id text NOT NULL,
  message_id text,
  uploader_id text NOT NULL,
  channel_id text NOT NULL,
  file_name text NOT NULL,
  file_size bigint NOT NULL,
  content_type text NOT NULL,
  storage_key text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (id),
  CONSTRAINT attachments_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT attachments_uploader_id_fkey
    FOREIGN KEY (uploader_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT attachments_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_attachments_message_id ON attachments (message_id);
CREATE INDEX idx_attachments_channel_id ON attachments (channel_id);
CREATE INDEX idx_attachments_uploader_id ON attachments (uploader_id);

ALTER TABLE attachments REPLICA IDENTITY FULL;
