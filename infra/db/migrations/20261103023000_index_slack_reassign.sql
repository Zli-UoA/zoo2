-- Slack アカウント連携の付け替え (slack::link::move_content) は、プレースホルダの投稿・添付を
-- 投稿者で引く。連携済みユーザーごとに投稿テーブルを全件走査しないよう、インデックスを足す。
CREATE INDEX idx_messages_user_id ON messages (user_id);
CREATE INDEX idx_slack_files_uploader_id ON slack_files (uploader_id);
